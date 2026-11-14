use std::collections::{BTreeSet, HashSet};

use crate::{
    catalog::Catalog,
    families::Families,
    game::Game,
    ids::{AppId, TagId},
    signals::Signals,
    tag_vector::{TagVector, dot, norm, tf_idf_vector},
    taste::Taste,
    votes::Votes,
};

// Tags that judge or package a game rather than say what it is.
const NOISE_TAGS: [TagId; 17] = [
    TagId(12095), // Sexual Content
    TagId(1756),  // Great Soundtrack
    TagId(6650),  // Nudity
    TagId(1693),  // Classic
    TagId(1669),  // Moddable
    TagId(7948),  // Soundtrack
    TagId(4711),  // Replay Value
    TagId(5708),  // Remake
    TagId(5411),  // Beautiful
    TagId(3934),  // Immersive
    TagId(4190),  // Addictive
    TagId(25085), // Touch-Friendly
    TagId(11123), // Mouse Only
    TagId(493),   // Early Access
    TagId(5230),  // Sequel
    TagId(3965),  // Epic
    TagId(7481),  // Controller
];

const SPECIFIC_TAGS: usize = 4;

pub fn specific_tags(game: &Game, catalog: &Catalog) -> Vec<String> {
    specific_tag_ids(game, catalog)
        .into_iter()
        .map(|id| match catalog.tags.get(&id) {
            Some(name) => name.english.clone(),
            None => format!("#{}", id.0),
        })
        .collect()
}

// The tags that say what the game is, whatever the profile.
pub fn specific_tag_ids(game: &Game, catalog: &Catalog) -> Vec<TagId> {
    let mut tags: Vec<_> = tf_idf_vector(&game.tags, &catalog.idf)
        .into_iter()
        .filter(|(id, _)| !NOISE_TAGS.contains(id))
        .collect();
    tags.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    tags.into_iter()
        .take(SPECIFIC_TAGS)
        .map(|(id, _)| id)
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

// One of the user's games, the ones Similar to can name.
#[derive(Debug, Clone)]
pub struct KnownGame {
    pub id: AppId,
    pub tag_vector: TagVector,
    pub norm: f64,
}

// How the user knows one of their games, in the order that decides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnownFrom {
    Steam,
    PlayedElsewhere,
    Library,
    WouldTry,
    Wishlist,
    Followed,
    Family(String),
}

// The games of every taste, each once, with the vector of the first taste it is in.
pub fn known_games<'a>(tastes: impl IntoIterator<Item = &'a Taste>) -> Vec<KnownGame> {
    let mut seen = HashSet::new();

    tastes
        .into_iter()
        .flat_map(|taste| &taste.games)
        .filter(|game| game.weight > 0.0 && !game.tag_vector.is_empty())
        .filter(|game| seen.insert(game.id))
        .map(|game| KnownGame {
            id: game.id,
            tag_vector: game.tag_vector.clone(),
            norm: norm(&game.tag_vector),
        })
        .collect()
}

pub fn known_from(
    id: AppId,
    signals: &Signals,
    votes: &Votes,
    played_elsewhere: &BTreeSet<AppId>,
    families: &Families,
) -> Option<KnownFrom> {
    let owned = || signals.library.iter().chain(&signals.borrowed);

    if owned().any(|game| game.id == id && game.total_hours > 0.0)
        || signals
            .recent
            .iter()
            .any(|game| game.id == id && game.total_hours > 0.0)
    {
        Some(KnownFrom::Steam)
    } else if played_elsewhere.contains(&id) {
        Some(KnownFrom::PlayedElsewhere)
    } else if owned().any(|game| game.id == id) {
        Some(KnownFrom::Library)
    } else if votes.get(&id).is_some_and(|vote| vote.kind.is_yes()) {
        Some(KnownFrom::WouldTry)
    } else if signals.wishlist.contains(&id) {
        Some(KnownFrom::Wishlist)
    } else if signals.followed.contains(&id) {
        Some(KnownFrom::Followed)
    } else {
        families
            .families
            .iter()
            .find(|family| family.games.contains(&id))
            .map(|family| KnownFrom::Family(family.name.clone()))
    }
}

// The closest of the user's games by cosine, however much it was played.
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
        .filter(|game| game.id != id)
        .map(|game| {
            (
                game,
                dot(tag_vector, &game.tag_vector) / (game_norm * game.norm),
            )
        })
        .filter(|&(_, similarity)| similarity > 0.0)
        .min_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)))
        .map(|(game, _)| game)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        catalog::{Freshness, TagName},
        currency::Currency,
        families::Family,
        game::{GameTag, Price, Reviews},
        region::Region,
        signals::{LibraryGame, RecentGame},
        taste::TasteGame,
        timestamp::Timestamp,
        votes::{Vote, VoteKind},
    };

    fn vector(pairs: &[(u32, f64)]) -> TagVector {
        pairs
            .iter()
            .map(|&(id, value)| (TagId(id), value))
            .collect()
    }

    type Tags<'a> = &'a [(u32, f64)];

    fn taste(games: &[(u32, f64, Tags)]) -> Taste {
        Taste {
            games: games
                .iter()
                .map(|&(id, weight, pairs)| TasteGame {
                    id: AppId(id),
                    weight,
                    tag_vector: vector(pairs),
                })
                .collect(),
        }
    }

    fn similar_id(id: u32, pairs: &[(u32, f64)], known: &[KnownGame]) -> Option<u32> {
        similar_to(AppId(id), &vector(pairs), known).map(|game| game.id.0)
    }

    #[test]
    fn the_closest_game_wins_even_with_fewer_hours() {
        let known = known_games([&taste(&[
            (1, 5.0, &[(1, 1.0), (2, 1.0)]),
            (2, 0.5, &[(1, 1.0)]),
        ])]);

        assert_eq!(similar_id(10, &[(1, 1.0)], &known), Some(2));
    }

    #[test]
    fn a_game_is_never_similar_to_itself() {
        let known = known_games([&taste(&[
            (1, 1.0, &[(1, 1.0)]),
            (2, 1.0, &[(1, 1.0), (2, 1.0)]),
        ])]);

        assert_eq!(similar_id(1, &[(1, 1.0)], &known), Some(2));
    }

    #[test]
    fn no_game_is_similar_without_a_shared_tag() {
        let known = known_games([&taste(&[(1, 1.0, &[(1, 1.0)])])]);

        assert_eq!(similar_id(10, &[(2, 1.0)], &known), None);
        assert_eq!(similar_id(10, &[], &known), None);
    }

    #[test]
    fn a_tie_goes_to_the_lowest_appid() {
        let known = known_games([&taste(&[(7, 1.0, &[(1, 1.0)]), (3, 2.0, &[(1, 2.0)])])]);

        assert_eq!(similar_id(10, &[(1, 1.0)], &known), Some(3));
    }

    #[test]
    fn a_game_of_a_family_can_be_similar_to_a_game_outside_it() {
        let signals = taste(&[(1, 1.0, &[(1, 1.0), (2, 1.0)])]);
        let family = taste(&[(2, 1.0, &[(1, 1.0), (3, 1.0)])]);
        let known = known_games([&signals, &family]);

        assert_eq!(similar_id(10, &[(1, 1.0), (2, 1.0)], &known), Some(1));
    }

    #[test]
    fn every_game_is_known_once_with_the_vector_of_its_first_taste() {
        let signals = taste(&[(1, 1.0, &[(1, 1.0)])]);
        let family = taste(&[(1, 1.0, &[(2, 1.0)]), (2, 1.0, &[(2, 1.0)])]);

        let known = known_games([&signals, &family]);

        let ids: Vec<u32> = known.iter().map(|game| game.id.0).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(known[0].tag_vector, vector(&[(1, 1.0)]));
    }

    #[test]
    fn a_game_with_no_weight_in_the_signals_is_known_only_from_a_family() {
        let signals = taste(&[(1, 0.0, &[(1, 1.0)])]);
        let family = taste(&[(1, 1.0, &[(1, 1.0)])]);

        assert!(known_games([&signals]).is_empty());
        assert_eq!(known_games([&signals, &family]).len(), 1);
    }

    fn library(id: u32, total_hours: f64) -> LibraryGame {
        LibraryGame {
            id: AppId(id),
            total_hours,
            last_played: None,
        }
    }

    // 1 played, 2 borrowed and played, 3 recent, 4 played elsewhere, 5 in the
    // library with no hours, 6 a Would try, 7 on the wishlist, 8 followed, 9
    // only in the families, 10 in the library for an hour and a Would try, 11
    // in the library with no hours and a Would try, 12 a Not for me.
    fn place(id: u32) -> Option<KnownFrom> {
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![
                library(1, 30.0),
                library(5, 0.0),
                library(10, 1.0),
                library(11, 0.0),
            ],
            borrowed: vec![library(2, 4.0)],
            recent: vec![RecentGame {
                id: AppId(3),
                total_hours: 2.0,
                recent_hours: 2.0,
            }],
            wishlist: vec![AppId(7)],
            followed: vec![AppId(7), AppId(8)],
        };
        let vote = |kind| Vote::new(kind, "2026-11-14");
        let votes = Votes::from([
            (AppId(6), vote(VoteKind::WouldTry)),
            (AppId(7), vote(VoteKind::NotForMe)),
            (AppId(10), vote(VoteKind::WouldTry)),
            (AppId(11), vote(VoteKind::WouldTry)),
            (AppId(12), vote(VoteKind::NotForMe)),
        ]);
        let family = |name: &str, games: &[u32]| Family {
            name: name.to_string(),
            games: games.iter().copied().map(AppId).collect(),
            automatic: None,
        };
        let families = Families {
            families: vec![family("Souls-like", &[1, 9]), family("Puzzle", &[9])],
            held_out: Vec::new(),
        };

        known_from(
            AppId(id),
            &signals,
            &votes,
            &BTreeSet::from([AppId(4), AppId(1)]),
            &families,
        )
    }

    #[test]
    fn a_game_played_on_steam_is_known_from_steam() {
        assert_eq!(place(1), Some(KnownFrom::Steam));
        assert_eq!(place(2), Some(KnownFrom::Steam));
        assert_eq!(place(3), Some(KnownFrom::Steam));
    }

    #[test]
    fn a_game_played_elsewhere_is_known_from_there() {
        assert_eq!(place(4), Some(KnownFrom::PlayedElsewhere));
    }

    #[test]
    fn a_game_in_the_library_with_no_hours_is_known_from_the_library() {
        assert_eq!(place(5), Some(KnownFrom::Library));
        assert_eq!(place(11), Some(KnownFrom::Library));
    }

    #[test]
    fn a_would_try_is_known_from_the_vote() {
        assert_eq!(place(6), Some(KnownFrom::WouldTry));
    }

    #[test]
    fn an_hour_on_steam_beats_a_would_try() {
        assert_eq!(place(10), Some(KnownFrom::Steam));
    }

    #[test]
    fn the_wishlist_comes_before_the_followed_games() {
        assert_eq!(place(7), Some(KnownFrom::Wishlist));
        assert_eq!(place(8), Some(KnownFrom::Followed));
    }

    #[test]
    fn a_game_only_in_the_families_is_known_from_the_first_one() {
        assert_eq!(place(9), Some(KnownFrom::Family("Souls-like".to_string())));
    }

    #[test]
    fn a_game_the_user_does_not_know_has_no_place() {
        assert_eq!(place(12), None);
    }

    fn tagged_game(tags: &[(u32, f64)]) -> Game {
        Game {
            id: AppId(1),
            name: "game".to_string(),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews {
                total: 100,
                positive: 90,
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

    // Each tag has its id over 10 as IDF, so a higher id is a rarer tag.
    fn tag_catalog(game: &Game, tags: BTreeMap<TagId, TagName>) -> Catalog {
        Catalog::new(
            vec![game.clone()],
            tags,
            game.tags
                .iter()
                .map(|tag| (tag.id, f64::from(tag.id.0) / 10.0))
                .collect(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn specific(tags: &[(u32, f64)]) -> Vec<u32> {
        let game = tagged_game(tags);

        specific_tag_ids(&game, &tag_catalog(&game, BTreeMap::new()))
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    #[test]
    fn the_specific_tags_go_by_tf_idf_not_by_weight() {
        assert_eq!(
            specific(&[(1, 1.0), (2, 0.9), (3, 0.8), (4, 0.7), (5, 0.6)]),
            vec![5, 4, 3, 2]
        );
    }

    #[test]
    fn a_tie_on_tf_idf_goes_to_the_lowest_tag_id() {
        assert_eq!(specific(&[(4, 0.5), (2, 1.0), (1, 1.0)]), vec![2, 4, 1]);
    }

    #[test]
    fn the_noise_tags_never_come_out_even_when_they_are_the_most_specific() {
        let mut tags: Vec<_> = NOISE_TAGS.iter().map(|id| (id.0, 1.0)).collect();
        tags.extend([(1, 1.0), (2, 1.0), (3, 1.0), (4, 1.0), (5, 1.0)]);

        assert_eq!(specific(&tags), vec![5, 4, 3, 2]);
    }

    #[test]
    fn a_game_with_few_useful_tags_gets_as_many_as_it_has() {
        assert_eq!(specific(&[(1, 1.0), (1756, 1.0), (2, 1.0)]), vec![2, 1]);
    }

    #[test]
    fn a_game_without_tags_gets_none() {
        assert!(specific(&[]).is_empty());
    }

    #[test]
    fn the_names_of_the_specific_tags_fall_back_to_the_id() {
        let game = tagged_game(&[(1, 1.0), (2, 1.0)]);
        let catalog = tag_catalog(
            &game,
            BTreeMap::from([(
                TagId(2),
                TagName {
                    english: "Puzzle".to_string(),
                    italian: "Rompicapo".to_string(),
                },
            )]),
        );

        assert_eq!(specific_tags(&game, &catalog), ["Puzzle", "#1"]);
    }
}
