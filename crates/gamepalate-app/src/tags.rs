use gamepalate_core::{
    effective_hours::hours_by_source,
    explanation::top_tag_ids,
    filters::hits_ban,
    ids::{AppId, TagId},
    search::SearchInputs,
    taste::signals_taste,
};

use crate::names::tag_name;

#[derive(Debug, Clone, PartialEq)]
pub struct GameTagsView {
    pub tags: Vec<TagOfGame>,
    pub weighted: bool,
    pub used: Vec<UsedTag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagOfGame {
    pub id: TagId,
    pub name: String,
    pub weight: f64,
    pub relative: f64,
    pub used: bool,
    pub banned: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsedTag {
    pub id: TagId,
    pub name: String,
    pub profile_games: usize,
    pub catalog_share: f64,
}

pub(crate) fn game_tags_view(id: AppId, inputs: &SearchInputs) -> Option<GameTagsView> {
    let catalog = inputs.catalog;
    let game = catalog.game(id)?;

    let hours = hours_by_source(
        inputs.signals,
        inputs.votes,
        &inputs.corrections.played_elsewhere,
        &inputs.config.signals,
    );
    let taste = signals_taste(
        &hours,
        catalog,
        inputs.off_catalog,
        inputs.corrections,
        &inputs.config.signals,
    );
    let used_ids = if taste.has_profile() {
        top_tag_ids(game, &taste.profile(), catalog)
    } else {
        Vec::new()
    };

    let used = used_ids
        .iter()
        .map(|&tag| UsedTag {
            id: tag,
            name: tag_name(tag, catalog),
            profile_games: taste
                .games
                .iter()
                .filter(|taste_game| taste_game.tag_vector.contains_key(&tag))
                .count(),
            catalog_share: (-catalog.idf.get(&tag).copied().unwrap_or(0.0)).exp(),
        })
        .collect();

    let mut sorted: Vec<_> = game.tags.iter().collect();
    sorted.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
    let first = sorted.first().map_or(0.0, |tag| tag.weight);
    let weighted = sorted.iter().any(|tag| tag.weight != first);

    let tags = sorted
        .iter()
        .map(|tag| TagOfGame {
            id: tag.id,
            name: tag_name(tag.id, catalog),
            weight: tag.weight,
            relative: if first > 0.0 { tag.weight / first } else { 0.0 },
            used: used_ids.contains(&tag.id),
            banned: inputs
                .corrections
                .banned_tags
                .get(&tag.id)
                .is_some_and(|&ban| hits_ban(game, tag.id, ban)),
        })
        .collect();

    Some(GameTagsView {
        tags,
        weighted,
        used,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gamepalate_core::{
        catalog::{Catalog, Freshness, TagName, compute_idf},
        config::Config,
        corrections::{Ban, Corrections},
        currency::Currency,
        families::Families,
        game::{Game, GameTag, Price, Reviews},
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, Signals},
        timestamp::Timestamp,
        votes::Votes,
    };

    use super::*;

    fn game(id: u32, tags: &[(u32, f64)]) -> Game {
        Game {
            id: AppId(id),
            name: format!("game {id}"),
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
                .map(|&(id, weight)| GameTag {
                    id: TagId(id),
                    weight,
                })
                .collect(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn catalog(games: Vec<Game>) -> Catalog {
        let idf = compute_idf(&games);
        Catalog::new(
            games,
            (1..=6)
                .map(|id| {
                    (
                        TagId(id),
                        TagName {
                            english: format!("Tag {id}"),
                            italian: format!("Tag {id}"),
                        },
                    )
                })
                .collect(),
            idf,
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn signals(played: &[u32]) -> Signals {
        Signals {
            read_at: Timestamp(0),
            library: played
                .iter()
                .map(|&id| LibraryGame {
                    id: AppId(id),
                    total_hours: 50.0,
                    last_played: None,
                })
                .collect(),
            borrowed: Vec::new(),
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        }
    }

    fn view_of(
        games: Vec<Game>,
        played: &[u32],
        banned_tags: BTreeMap<TagId, Ban>,
        id: u32,
    ) -> Option<GameTagsView> {
        let catalog = catalog(games);
        let corrections = Corrections {
            banned_tags,
            ..Corrections::default()
        };
        game_tags_view(
            AppId(id),
            &SearchInputs {
                catalog: &catalog,
                game_vectors: &[],
                signals: &signals(played),
                votes: &Votes::new(),
                corrections: &corrections,
                families: &Families {
                    families: Vec::new(),
                    held_out: Vec::new(),
                },
                off_catalog: &OffCatalogGames::new(),
                config: &Config::default(),
            },
        )
    }

    fn catalog_games() -> Vec<Game> {
        vec![
            game(1, &[(3, 0.15), (1, 0.4), (2, 0.2), (5, 0.1), (4, 0.1)]),
            game(2, &[(1, 1.0)]),
            game(3, &[(6, 1.0)]),
            game(4, &[(6, 1.0)]),
        ]
    }

    fn used_ids(view: &GameTagsView) -> Vec<u32> {
        view.used.iter().map(|tag| tag.id.0).collect()
    }

    #[test]
    fn tags_go_by_decreasing_weight_then_id() {
        let view = view_of(catalog_games(), &[], BTreeMap::new(), 1).unwrap();

        let ids: Vec<u32> = view.tags.iter().map(|tag| tag.id.0).collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_relative_weight_is_against_the_first_tag() {
        let view = view_of(catalog_games(), &[], BTreeMap::new(), 1).unwrap();

        assert_eq!(view.tags[0].relative, 1.0);
        assert_eq!(view.tags[1].relative, 0.5);
        assert_eq!(view.tags[3].relative, 0.25);
        assert!(view.weighted);
    }

    #[test]
    fn a_single_tag_or_equal_weights_are_not_weighted() {
        let single = view_of(catalog_games(), &[], BTreeMap::new(), 2).unwrap();
        assert!(!single.weighted);

        let equal = view_of(
            vec![game(1, &[(1, 0.5), (2, 0.5)])],
            &[],
            BTreeMap::new(),
            1,
        )
        .unwrap();
        assert!(!equal.weighted);
    }

    #[test]
    fn used_tags_are_the_ones_of_top_tags_in_the_same_order() {
        let games = vec![
            game(1, &[(1, 0.4), (2, 0.3), (3, 0.2), (4, 0.1)]),
            game(2, &[(1, 0.1), (2, 0.2), (3, 0.3), (4, 0.4)]),
            game(3, &[(5, 1.0)]),
            game(4, &[(6, 1.0)]),
            game(5, &[(6, 1.0)]),
        ];
        let catalog = catalog(games.clone());
        let view = view_of(games, &[2], BTreeMap::new(), 1).unwrap();
        let hours = hours_by_source(
            &signals(&[2]),
            &Votes::new(),
            &Default::default(),
            &Config::default().signals,
        );
        let taste = signals_taste(
            &hours,
            &catalog,
            &OffCatalogGames::new(),
            &Corrections::default(),
            &Config::default().signals,
        );
        let expected: Vec<u32> =
            top_tag_ids(catalog.game(AppId(1)).unwrap(), &taste.profile(), &catalog)
                .iter()
                .map(|tag| tag.0)
                .collect();

        assert_eq!(expected.len(), 3);
        assert_eq!(used_ids(&view), expected);
        assert_eq!(view.tags.iter().filter(|tag| tag.used).count(), 3);
    }

    #[test]
    fn fewer_than_three_used_tags_are_listed_as_they_are() {
        let one = view_of(catalog_games(), &[2], BTreeMap::new(), 1).unwrap();
        assert_eq!(used_ids(&one), vec![1]);

        let none = view_of(catalog_games(), &[3], BTreeMap::new(), 1).unwrap();
        assert!(none.used.is_empty());
        assert!(none.tags.iter().all(|tag| !tag.used));
    }

    #[test]
    fn a_used_tag_counts_profile_games_and_catalog_share() {
        let view = view_of(catalog_games(), &[2], BTreeMap::new(), 1).unwrap();
        let used = &view.used[0];

        assert_eq!(used.id, TagId(1));
        assert_eq!(used.profile_games, 1);
        assert!((used.catalog_share - 3.0 / 4.0).abs() < 1e-9);
    }

    #[test]
    fn without_a_profile_nothing_is_used() {
        let view = view_of(catalog_games(), &[], BTreeMap::new(), 1).unwrap();

        assert!(view.used.is_empty());
    }

    #[test]
    fn a_ban_on_the_top_tags_only_hits_the_ones_inside() {
        let bans = BTreeMap::from([(TagId(2), Ban::TopTags(2)), (TagId(3), Ban::TopTags(2))]);
        let view = view_of(catalog_games(), &[], bans, 1).unwrap();
        let banned = |id: u32| view.tags.iter().find(|tag| tag.id.0 == id).unwrap().banned;

        assert!(banned(2));
        assert!(!banned(3));
        assert!(!banned(1));
    }

    #[test]
    fn a_ban_everywhere_hits_the_tag_wherever_it_is() {
        let bans = BTreeMap::from([(TagId(5), Ban::Everywhere)]);
        let view = view_of(catalog_games(), &[], bans, 1).unwrap();

        let tag = view.tags.iter().find(|tag| tag.id == TagId(5)).unwrap();
        assert!(tag.banned);
    }

    #[test]
    fn a_game_outside_the_catalog_has_no_view() {
        assert_eq!(view_of(catalog_games(), &[], BTreeMap::new(), 99), None);
    }
}
