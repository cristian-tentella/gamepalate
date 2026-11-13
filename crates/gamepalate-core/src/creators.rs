use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::Catalog, config::SignalsConfig, effective_hours::hours_by_source, game::Game,
    ids::AppId, signals::Signals, votes::Votes,
};

pub fn normalized_name(name: &str) -> String {
    let name = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    match name.strip_suffix('.') {
        Some(name) => name.trim_end().to_string(),
        None => name,
    }
}

fn normalized_names(names: &[String]) -> BTreeSet<String> {
    names
        .iter()
        .map(|name| normalized_name(name))
        .filter(|name| !name.is_empty())
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LikedCreators {
    pub games: BTreeSet<AppId>,
    pub developers: BTreeMap<String, usize>,
    pub franchises: BTreeMap<String, usize>,
}

impl LikedCreators {
    pub fn has_liked_developer(&self, game: &Game) -> bool {
        self.has_liked(game, &game.developers, &self.developers)
    }

    pub fn has_liked_franchise(&self, game: &Game) -> bool {
        self.has_liked(game, &game.franchises, &self.franchises)
    }

    fn has_liked(&self, game: &Game, names: &[String], counts: &BTreeMap<String, usize>) -> bool {
        let own = usize::from(self.games.contains(&game.id));

        normalized_names(names)
            .iter()
            .any(|name| counts.get(name).is_some_and(|&count| count > own))
    }
}

pub fn liked_creators(
    signals: &Signals,
    played_elsewhere: &BTreeSet<AppId>,
    signals_config: &SignalsConfig,
    catalog: &Catalog,
) -> LikedCreators {
    let mut liked = LikedCreators::default();

    let hours = hours_by_source(signals, &Votes::new(), played_elsewhere, signals_config);
    for (&id, &(hours, _)) in &hours {
        if hours < signals_config.min_hours {
            continue;
        }
        let Some(game) = catalog.game(id) else {
            continue;
        };

        liked.games.insert(id);
        for name in normalized_names(&game.developers) {
            *liked.developers.entry(name).or_insert(0) += 1;
        }
        for name in normalized_names(&game.franchises) {
            *liked.franchises.entry(name).or_insert(0) += 1;
        }
    }

    liked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::Freshness,
        currency::Currency,
        game::{Price, Reviews},
        region::Region,
        signals::LibraryGame,
        timestamp::Timestamp,
        votes::{Vote, VoteKind},
    };

    fn game(id: u32, developers: &[&str], franchises: &[&str]) -> Game {
        Game {
            id: AppId(id),
            name: format!("game {id}"),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews {
                total: 100,
                positive: 90,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: developers.iter().map(|name| name.to_string()).collect(),
            franchises: franchises.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn catalog(games: Vec<Game>) -> Catalog {
        Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn played(id: u32, total_hours: f64) -> LibraryGame {
        LibraryGame {
            id: AppId(id),
            total_hours,
            last_played: None,
        }
    }

    fn signals(library: Vec<LibraryGame>, wishlist: &[u32]) -> Signals {
        Signals {
            read_at: Timestamp(0),
            library,
            borrowed: Vec::new(),
            recent: Vec::new(),
            wishlist: wishlist.iter().map(|&id| AppId(id)).collect(),
            followed: Vec::new(),
        }
    }

    fn liked(signals: &Signals, catalog: &Catalog) -> LikedCreators {
        liked_creators(
            signals,
            &BTreeSet::new(),
            &SignalsConfig::default(),
            catalog,
        )
    }

    #[test]
    fn names_meet_whatever_their_case_spaces_and_final_dot() {
        assert_eq!(
            normalized_name("Square Enix"),
            normalized_name("SQUARE ENIX ")
        );
        assert_eq!(
            normalized_name("Supergiant Games, Inc."),
            normalized_name("Supergiant Games, Inc")
        );
        assert_eq!(
            normalized_name("  Double   Fine \t Productions"),
            "double fine productions"
        );
    }

    #[test]
    fn a_game_played_too_little_likes_nothing_but_one_in_the_wishlist_does() {
        let catalog = catalog(vec![
            game(1, &["Little"], &["Short"]),
            game(2, &["Wished"], &["Hoped"]),
            game(10, &["Little"], &["Short"]),
            game(11, &["Wished"], &["Hoped"]),
        ]);
        let liked = liked(&signals(vec![played(1, 1.0)], &[2]), &catalog);

        let new_game = |id| catalog.game(AppId(id)).unwrap();
        assert!(!liked.has_liked_developer(new_game(10)));
        assert!(!liked.has_liked_franchise(new_game(10)));
        assert!(liked.has_liked_developer(new_game(11)));
        assert!(liked.has_liked_franchise(new_game(11)));
    }

    #[test]
    fn a_would_try_vote_likes_nothing() {
        let catalog = catalog(vec![
            game(1, &["Voted"], &["Voted"]),
            game(10, &["Voted"], &["Voted"]),
        ]);
        let signals = signals(Vec::new(), &[]);
        let votes = Votes::from([(AppId(1), Vote::new(VoteKind::WouldTry, "2026-11-08"))]);

        let liked = liked(&signals, &catalog);

        assert!(
            hours_by_source(
                &signals,
                &votes,
                &BTreeSet::new(),
                &SignalsConfig::default()
            )
            .contains_key(&AppId(1))
        );
        assert_eq!(liked, LikedCreators::default());
        assert!(!liked.has_liked_developer(catalog.game(AppId(10)).unwrap()));
    }

    #[test]
    fn a_liked_game_does_not_like_itself_until_a_second_game_shares_the_name() {
        let alone = catalog(vec![game(1, &["Studio", "Studio."], &["Saga"])]);
        let liked_alone = liked(&signals(vec![played(1, 30.0)], &[]), &alone);

        assert_eq!(liked_alone.developers["studio"], 1);
        assert!(!liked_alone.has_liked_developer(alone.game(AppId(1)).unwrap()));
        assert!(!liked_alone.has_liked_franchise(alone.game(AppId(1)).unwrap()));

        let together = catalog(vec![
            game(1, &["Studio", "Studio."], &["Saga"]),
            game(2, &["STUDIO"], &["saga"]),
        ]);
        let liked_together = liked(
            &signals(vec![played(1, 30.0), played(2, 10.0)], &[]),
            &together,
        );

        assert!(liked_together.has_liked_developer(together.game(AppId(1)).unwrap()));
        assert!(liked_together.has_liked_franchise(together.game(AppId(1)).unwrap()));
    }

    #[test]
    fn a_played_game_out_of_the_catalog_brings_no_names() {
        let catalog = catalog(vec![game(10, &["Gone"], &[])]);
        let liked = liked(&signals(vec![played(99, 30.0)], &[]), &catalog);

        assert_eq!(liked, LikedCreators::default());
        assert!(!liked.has_liked_developer(catalog.game(AppId(10)).unwrap()));
    }
}
