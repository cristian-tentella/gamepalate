use std::collections::HashMap;

use crate::{
    affinity::{affinity_values, halves, profile_similarity},
    game_vector::GameVector,
    ids::AppId,
    opinion::held_out_opinions,
    ranking::relevance,
    search::{SearchInputs, search_tastes},
    tag_vector::{add_weighted, norm},
    taste::Taste,
};

pub fn best_matches(inputs: &SearchInputs) -> HashMap<AppId, f64> {
    let Some(signals) = search_tastes(inputs, None)
        .ok()
        .and_then(|tastes| tastes.into_iter().next())
    else {
        return HashMap::new();
    };
    let voted: Vec<usize> = inputs
        .votes
        .keys()
        .filter_map(|&id| {
            inputs
                .game_vectors
                .binary_search_by_key(&id, |game_vector| game_vector.id)
                .ok()
        })
        .collect();
    let Some(affinities) = held_out_affinities(inputs.game_vectors, &signals.taste, &voted) else {
        return HashMap::new();
    };
    let relevance_config = &inputs.config.relevance;
    let opinions = if relevance_config.votes_weight > 0.0 {
        held_out_opinions(inputs.votes, inputs.catalog, &inputs.liked_creators())
    } else {
        HashMap::new()
    };

    voted
        .iter()
        .zip(affinities)
        .map(|(&i, affinity)| {
            let game_vector = &inputs.game_vectors[i];
            let opinion = opinions.get(&game_vector.id).copied().unwrap_or(0.0);
            let score = relevance(
                affinity,
                game_vector.quality,
                relevance_config.quality_weight,
            ) * (relevance_config.votes_weight * opinion).exp();
            (game_vector.id, score)
        })
        .collect()
}

fn held_out_affinities(
    game_vectors: &[GameVector],
    taste: &Taste,
    voted: &[usize],
) -> Option<Vec<f64>> {
    let mut halves = halves(game_vectors, taste)?;
    let profile = taste.profile();

    for &i in voted {
        let game_vector = &game_vectors[i];
        let Some(taste_game) = taste.games.iter().find(|game| game.id == game_vector.id) else {
            continue;
        };
        let mut held_out_profile = profile.clone();
        add_weighted(
            &mut held_out_profile,
            &taste_game.tag_vector,
            -taste_game.weight,
        );
        let held_out_norm = norm(&held_out_profile);
        halves.profile[i] = if held_out_norm == 0.0 {
            0.0
        } else {
            profile_similarity(game_vector, &held_out_profile, held_out_norm)
        };
    }

    let affinities = affinity_values(&halves.profile, &halves.references);

    Some(voted.iter().map(|&i| affinities[i]).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        catalog::{Catalog, Freshness},
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::Families,
        game::{Game, GameTag, Price, Reviews},
        game_vector::game_vectors,
        ids::TagId,
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, Signals},
        timestamp::Timestamp,
        votes::{Vote, VoteKind, Votes},
    };

    fn game(id: u32, tags: &[u32]) -> Game {
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
                .map(|&tag| GameTag {
                    id: TagId(tag),
                    weight: 1.0,
                })
                .collect(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn would_try() -> Vote {
        Vote::new(VoteKind::WouldTry, "2026-11-02")
    }

    fn matches_of(games: Vec<Game>, votes: &Votes) -> HashMap<AppId, f64> {
        let catalog = Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 1.0), (TagId(2), 1.0), (TagId(3), 1.0)]),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let vectors = game_vectors(&catalog);
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 20.0,
                last_played: None,
            }],
            borrowed: Vec::new(),
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };

        best_matches(&SearchInputs {
            catalog: &catalog,
            game_vectors: &vectors,
            signals: &signals,
            votes,
            corrections: &Corrections::default(),
            families: &Families::default(),
            off_catalog: &OffCatalogGames::new(),
            config: &Config::default(),
        })
    }

    #[test]
    fn a_would_try_scores_nothing_from_its_own_tags() {
        let votes = Votes::from([(AppId(10), would_try()), (AppId(11), would_try())]);

        let matches = matches_of(
            vec![
                game(1, &[1]),
                game(2, &[2]),
                game(10, &[3]),
                game(11, &[1, 2]),
            ],
            &votes,
        );

        assert_eq!(matches[&AppId(10)], 0.0);
        assert!(matches[&AppId(11)] > 0.0);
    }

    #[test]
    fn a_would_try_closer_to_the_taste_comes_first() {
        let votes = Votes::from([(AppId(10), would_try()), (AppId(11), would_try())]);

        let matches = matches_of(
            vec![
                game(1, &[1]),
                game(2, &[3]),
                game(10, &[1]),
                game(11, &[1, 2]),
            ],
            &votes,
        );

        assert!(matches[&AppId(10)] > matches[&AppId(11)]);
    }

    #[test]
    fn a_voted_game_out_of_the_catalog_has_no_score() {
        let votes = Votes::from([(AppId(10), would_try()), (AppId(99), would_try())]);

        let matches = matches_of(vec![game(1, &[1]), game(10, &[1])], &votes);

        assert!(matches.contains_key(&AppId(10)));
        assert!(!matches.contains_key(&AppId(99)));
    }
}
