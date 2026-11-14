use std::collections::BTreeMap;

use crate::{
    catalog::Catalog,
    effective_hours::HoursSource,
    game::Game,
    ids::{AppId, TagId},
    tag_vector::{TagVector, dot, norm, tf_idf_vector},
    taste::Taste,
};

pub fn top_tags(game: &Game, profile: &TagVector, catalog: &Catalog) -> Vec<String> {
    top_tag_ids(game, profile, catalog)
        .into_iter()
        .map(|id| match catalog.tags.get(&id) {
            Some(name) => name.english.clone(),
            None => format!("#{}", id.0),
        })
        .collect()
}

pub fn top_tag_ids(game: &Game, profile: &TagVector, catalog: &Catalog) -> Vec<TagId> {
    let vector = tf_idf_vector(&game.tags, &catalog.idf);
    let mut contributions: Vec<_> = vector
        .iter()
        .map(|(&id, &value)| (id, value * profile.get(&id).copied().unwrap_or(0.0)))
        .filter(|&(_, contribution)| contribution > 0.0)
        .collect();
    contributions.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    contributions.iter().take(3).map(|&(id, _)| id).collect()
}

// A game of the user's that a recommended game can be similar to.
#[derive(Debug, Clone)]
pub struct KnownGame {
    pub id: AppId,
    pub tag_vector: TagVector,
    pub norm: f64,
    // None for a family game with no hours, vote or wishlist behind it
    pub source: Option<HoursSource>,
}

// The games of every taste once each, the first taste winning.
pub fn known_games<'a>(
    tastes: impl IntoIterator<Item = &'a Taste>,
    sources: &BTreeMap<AppId, (f64, HoursSource)>,
) -> Vec<KnownGame> {
    let mut known: BTreeMap<AppId, KnownGame> = BTreeMap::new();

    for taste in tastes {
        for game in &taste.games {
            if game.weight <= 0.0 || game.tag_vector.is_empty() {
                continue;
            }
            known.entry(game.id).or_insert_with(|| KnownGame {
                id: game.id,
                tag_vector: game.tag_vector.clone(),
                norm: norm(&game.tag_vector),
                source: sources.get(&game.id).map(|&(_, source)| source),
            });
        }
    }

    known.into_values().collect()
}

// The most similar of the known games, never the game itself.
pub fn similar_to<'a>(
    id: AppId,
    tag_vector: &TagVector,
    known: &'a [KnownGame],
) -> Option<&'a KnownGame> {
    let game_norm = norm(tag_vector);
    if game_norm == 0.0 {
        return None;
    }

    known
        .iter()
        .filter(|other| other.id != id && other.norm > 0.0)
        .map(|other| {
            let similarity = dot(tag_vector, &other.tag_vector) / (game_norm * other.norm);
            (other, similarity)
        })
        .filter(|&(_, similarity)| similarity > 0.0)
        .min_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)))
        .map(|(other, _)| other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::TasteGame;

    fn vector(pairs: &[(u32, f64)]) -> TagVector {
        pairs
            .iter()
            .map(|&(id, value)| (TagId(id), value))
            .collect()
    }

    fn taste_game(id: u32, weight: f64, pairs: &[(u32, f64)]) -> TasteGame {
        TasteGame {
            id: AppId(id),
            weight,
            tag_vector: vector(pairs),
        }
    }

    fn known(pairs: &[(u32, &[(u32, f64)])]) -> Vec<KnownGame> {
        let taste = Taste {
            games: pairs
                .iter()
                .map(|&(id, tags)| taste_game(id, 1.0, tags))
                .collect(),
        };
        known_games([&taste], &BTreeMap::new())
    }

    fn similar(tags: &[(u32, f64)], known: &[KnownGame]) -> Option<u32> {
        similar_to(AppId(100), &vector(tags), known).map(|game| game.id.0)
    }

    #[test]
    fn similar_to_picks_the_nearest_game() {
        let known = known(&[(1, &[(1, 1.0)]), (2, &[(1, 1.0), (2, 1.0)])]);

        assert_eq!(similar(&[(1, 1.0), (2, 0.9)], &known), Some(2));
        assert_eq!(similar(&[(1, 1.0)], &known), Some(1));
    }

    #[test]
    fn similar_to_never_picks_the_game_itself() {
        let known = known(&[(100, &[(1, 1.0)]), (2, &[(1, 1.0), (2, 1.0)])]);

        assert_eq!(similar(&[(1, 1.0)], &known), Some(2));
    }

    #[test]
    fn similar_to_needs_a_shared_tag() {
        let known = known(&[(1, &[(1, 1.0)])]);

        assert_eq!(similar(&[(2, 1.0)], &known), None);
        assert_eq!(similar(&[], &known), None);
    }

    #[test]
    fn similar_to_breaks_ties_with_the_lowest_appid() {
        let known = known(&[(3, &[(1, 1.0)]), (2, &[(1, 2.0)])]);

        assert_eq!(similar(&[(1, 1.0)], &known), Some(2));
    }

    #[test]
    fn known_games_take_each_game_once_with_its_source() {
        let signals = Taste {
            games: vec![
                taste_game(1, 2.0, &[(1, 1.0)]),
                taste_game(2, 0.0, &[(1, 1.0)]),
            ],
        };
        let family = Taste {
            games: vec![
                taste_game(1, 1.0, &[(2, 1.0)]),
                taste_game(3, 1.0, &[(2, 1.0)]),
            ],
        };
        let sources = BTreeMap::from([(AppId(1), (10.0, HoursSource::WouldTry))]);

        let known = known_games([&signals, &family], &sources);

        let summary: Vec<(u32, Option<HoursSource>)> =
            known.iter().map(|game| (game.id.0, game.source)).collect();
        assert_eq!(summary, [(1, Some(HoursSource::WouldTry)), (3, None)]);
        assert_eq!(known[0].tag_vector, vector(&[(1, 1.0)]));
    }
}
