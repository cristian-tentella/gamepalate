use std::collections::{BTreeMap, BTreeSet};

use crate::{
    catalog::{Catalog, TagName},
    families::Family,
    ids::{AppId, TagId},
    signals::Signals,
    tag_vector::{TagVector, add_weighted, dot, norm},
    taste::Taste,
};

const MIN_GAMES: usize = 6;
const MIN_GROUPS: usize = 2;
const MAX_GROUPS: usize = 8;
const MIN_GROUP_GAMES: usize = 3;
const FAMILY_GAMES: usize = 8;
const MAX_ROUNDS: usize = 30;
const NAME_TAGS: usize = 3;
const STARTS: usize = 8;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

struct PlayedGame {
    id: AppId,
    weight: f64,
    hours: f64,
    unit: TagVector,
}

// Groups the played games of the Signals taste by their tags, one Family per group.
pub fn automatic_families(taste: &Taste, signals: &Signals, catalog: &Catalog) -> Vec<Family> {
    let games = played_games(taste, signals, catalog);
    if games.len() < MIN_GAMES {
        return Vec::new();
    }

    let count = group_count(games.len());
    let mut random = Xorshift(SEED);
    let mut best: Option<(f64, Vec<usize>, Vec<TagVector>)> = None;
    for _ in 0..STARTS {
        let mut centers = first_centers(&games, count, &mut random);
        let groups = assign_groups(&games, &mut centers);
        let score = fit(&games, &groups, &centers);
        if best.as_ref().is_none_or(|(best, _, _)| score > *best) {
            best = Some((score, groups, centers));
        }
    }

    let Some((_, groups, centers)) = best else {
        return Vec::new();
    };
    families(&games, &groups, &centers, &catalog.tags)
}

pub fn group_count(games: usize) -> usize {
    ((games as f64).sqrt() / 2.0)
        .round()
        .clamp(MIN_GROUPS as f64, MAX_GROUPS as f64) as usize
}

// The hours that `hours_by_source` counts as played, without the recent multiplier.
fn played_hours(signals: &Signals) -> BTreeMap<AppId, f64> {
    let mut hours: BTreeMap<AppId, f64> = signals
        .library
        .iter()
        .map(|game| (game.id, game.total_hours))
        .collect();

    for game in &signals.borrowed {
        hours.entry(game.id).or_insert(game.total_hours);
    }
    for game in &signals.recent {
        hours.entry(game.id).or_insert(game.total_hours);
    }

    hours
}

// Only Catalog games: the ones off it are mostly free to play and make groups of their own.
fn played_games(taste: &Taste, signals: &Signals, catalog: &Catalog) -> Vec<PlayedGame> {
    let hours = played_hours(signals);

    let mut games: Vec<PlayedGame> = taste
        .games
        .iter()
        .filter(|game| catalog.game(game.id).is_some())
        .filter_map(|game| {
            let hours = *hours.get(&game.id)?;
            let unit = unit(&game.tag_vector)?;
            Some(PlayedGame {
                id: game.id,
                weight: game.weight,
                hours,
                unit,
            })
        })
        .collect();

    games.sort_by_key(|game| game.id);
    games.dedup_by_key(|game| game.id);
    games
}

fn unit(tag_vector: &TagVector) -> Option<TagVector> {
    let length = norm(tag_vector);
    if length == 0.0 {
        return None;
    }

    Some(
        tag_vector
            .iter()
            .map(|(&tag, &value)| (tag, value / length))
            .collect(),
    )
}

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn pick(&mut self, odds: &[f64]) -> Option<usize> {
        let total: f64 = odds.iter().sum();
        if total <= 0.0 {
            return None;
        }

        let mut target = self.next() * total;
        for (i, &odd) in odds.iter().enumerate() {
            target -= odd;
            if target < 0.0 {
                return Some(i);
            }
        }
        odds.iter().rposition(|&odd| odd > 0.0)
    }
}

// k-means++: each center is drawn with odds of weight × distance from the centers chosen so far.
fn first_centers(games: &[PlayedGame], count: usize, random: &mut Xorshift) -> Vec<TagVector> {
    let mut chosen: Vec<usize> = Vec::new();

    while chosen.len() < count.min(games.len()) {
        let odds: Vec<f64> = games
            .iter()
            .enumerate()
            .map(|(i, game)| {
                if chosen.contains(&i) {
                    return 0.0;
                }
                let closest = chosen
                    .iter()
                    .map(|&c| dot(&game.unit, &games[c].unit))
                    .fold(0.0, f64::max);
                game.weight * (1.0 - closest).max(0.0)
            })
            .collect();

        let next = random
            .pick(&odds)
            .or_else(|| (0..games.len()).find(|i| !chosen.contains(i)));
        let Some(i) = next else {
            break;
        };
        chosen.push(i);
    }

    chosen.iter().map(|&i| games[i].unit.clone()).collect()
}

fn fit(games: &[PlayedGame], groups: &[usize], centers: &[TagVector]) -> f64 {
    games
        .iter()
        .zip(groups)
        .map(|(game, &group)| game.weight * dot(&game.unit, &centers[group]))
        .sum()
}

fn assign_groups(games: &[PlayedGame], centers: &mut [TagVector]) -> Vec<usize> {
    let mut groups = vec![usize::MAX; games.len()];

    for _ in 0..MAX_ROUNDS {
        let new_groups: Vec<usize> = games
            .iter()
            .map(|game| closest_center(&game.unit, centers))
            .collect();
        if new_groups == groups {
            break;
        }
        groups = new_groups;

        for (i, center) in centers.iter_mut().enumerate() {
            let mut sum = TagVector::new();
            for (game, _) in games.iter().zip(&groups).filter(|&(_, &group)| group == i) {
                add_weighted(&mut sum, &game.unit, game.weight);
            }
            if let Some(new_center) = unit(&sum) {
                *center = new_center;
            }
        }
    }

    groups
}

fn closest_center(unit: &TagVector, centers: &[TagVector]) -> usize {
    let mut best = 0;
    let mut best_cosine = f64::NEG_INFINITY;

    for (i, center) in centers.iter().enumerate() {
        let cosine = dot(unit, center);
        if cosine > best_cosine {
            best = i;
            best_cosine = cosine;
        }
    }

    best
}

fn families(
    games: &[PlayedGame],
    groups: &[usize],
    centers: &[TagVector],
    tag_names: &BTreeMap<TagId, TagName>,
) -> Vec<Family> {
    let members: Vec<Vec<&PlayedGame>> = (0..centers.len())
        .map(|i| {
            let mut members: Vec<&PlayedGame> = games
                .iter()
                .zip(groups)
                .filter(|&(_, &group)| group == i)
                .map(|(game, _)| game)
                .collect();
            members.sort_by(|a, b| b.hours.total_cmp(&a.hours).then(a.id.cmp(&b.id)));
            members
        })
        .collect();

    let non_empty: Vec<usize> = (0..centers.len())
        .filter(|&i| !members[i].is_empty())
        .collect();

    let mut kept: Vec<(f64, String, Vec<AppId>)> = (0..centers.len())
        .filter(|&i| members[i].len() >= MIN_GROUP_GAMES)
        .map(|i| {
            let weight = members[i].iter().map(|game| game.weight).sum();
            let games = members[i]
                .iter()
                .take(FAMILY_GAMES)
                .map(|game| game.id)
                .collect();
            let others: Vec<&TagVector> = non_empty
                .iter()
                .filter(|&&j| j != i)
                .map(|&j| &centers[j])
                .collect();
            (weight, group_name(&centers[i], &others, tag_names), games)
        })
        .collect();

    kept.sort_by(|a, b| b.0.total_cmp(&a.0));

    let names = unique_names(kept.iter().map(|(_, name, _)| name.clone()).collect());
    kept.into_iter()
        .zip(names)
        .map(|((_, _, games), name)| Family { name, games })
        .collect()
}

// The tags that weigh more in this center than in the others; with no others, the heaviest.
fn group_name(
    center: &TagVector,
    others: &[&TagVector],
    tag_names: &BTreeMap<TagId, TagName>,
) -> String {
    let mut scores: Vec<(TagId, f64)> = center
        .iter()
        .filter(|&(_, &value)| value > 0.0)
        .map(|(&tag, &value)| {
            if others.is_empty() {
                return (tag, value);
            }
            let elsewhere: f64 = others
                .iter()
                .map(|other| other.get(&tag).copied().unwrap_or(0.0))
                .sum();
            (tag, value - elsewhere / others.len() as f64)
        })
        .collect();

    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    scores
        .iter()
        .take(NAME_TAGS)
        .map(|(tag, _)| {
            tag_names
                .get(tag)
                .map_or_else(|| format!("#{}", tag.0), |name| name.english.clone())
        })
        .collect::<Vec<String>>()
        .join(" · ")
}

fn unique_names(names: Vec<String>) -> Vec<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();

    names
        .into_iter()
        .map(|name| {
            let mut unique = name.clone();
            let mut number = 2;
            while !used.insert(unique.clone()) {
                unique = format!("{name} {number}");
                number += 1;
            }
            unique
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{Catalog, Freshness},
        config::SignalsConfig,
        corrections::Corrections,
        currency::Currency,
        effective_hours::hours_by_source,
        game::{Game, GameTag, Price, Reviews},
        off_catalog::OffCatalogGame,
        region::Region,
        signals::LibraryGame,
        taste::signals_taste,
        timestamp::Timestamp,
        votes::Votes,
    };

    type Tags<'a> = &'a [(u32, f64)];

    struct Library {
        catalog: Catalog,
        signals: Signals,
    }

    // Every tag has idf 1 and is called "tag <id>".
    fn library(played: &[(u32, f64, Tags)], wishlist: &[(u32, Tags)]) -> Library {
        let game = |id: u32, tags: Tags| Game {
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
            tags: tags
                .iter()
                .map(|&(id, weight)| GameTag {
                    id: TagId(id),
                    weight,
                })
                .collect(),
            developers: Vec::new(),
            franchises: Vec::new(),
        };

        let games: Vec<Game> = played
            .iter()
            .map(|&(id, _, tags)| game(id, tags))
            .chain(wishlist.iter().map(|&(id, tags)| game(id, tags)))
            .collect();
        let tag_ids: BTreeSet<TagId> = games
            .iter()
            .flat_map(|game| game.tags.iter().map(|tag| tag.id))
            .collect();

        Library {
            catalog: Catalog::new(
                games,
                tag_ids
                    .iter()
                    .map(|&id| {
                        (
                            id,
                            TagName {
                                english: format!("tag {}", id.0),
                                italian: format!("etichetta {}", id.0),
                            },
                        )
                    })
                    .collect(),
                tag_ids.iter().map(|&id| (id, 1.0)).collect(),
                Freshness {
                    prices_updated_at: Timestamp(0),
                    tags_updated_at: Timestamp(0),
                    region: Region::ITALY,
                },
                Currency::italian(),
            ),
            signals: Signals {
                read_at: Timestamp(0),
                library: played
                    .iter()
                    .map(|&(id, total_hours, _)| LibraryGame {
                        id: AppId(id),
                        total_hours,
                        last_played: None,
                    })
                    .collect(),
                borrowed: Vec::new(),
                recent: Vec::new(),
                wishlist: wishlist.iter().map(|&(id, _)| AppId(id)).collect(),
                followed: Vec::new(),
            },
        }
    }

    fn taste(library: &Library, corrections: &Corrections) -> Taste {
        let config = SignalsConfig::default();
        let hours = hours_by_source(&library.signals, &Votes::new(), &BTreeSet::new(), &config);
        signals_taste(
            &hours,
            &library.catalog,
            &BTreeMap::new(),
            corrections,
            &config,
        )
    }

    fn families_of(library: &Library, corrections: &Corrections) -> Vec<Family> {
        automatic_families(
            &taste(library, corrections),
            &library.signals,
            &library.catalog,
        )
    }

    fn ids(ids: &[u32]) -> Vec<AppId> {
        ids.iter().copied().map(AppId).collect()
    }

    fn sorted(games: &[AppId]) -> Vec<AppId> {
        let mut games = games.to_vec();
        games.sort();
        games
    }

    const PUZZLE: Tags = &[(1, 1.0), (2, 0.5)];
    const FIGHTING: Tags = &[(3, 1.0), (4, 0.5)];
    const FARMING: Tags = &[(5, 1.0), (6, 0.5)];

    #[test]
    fn group_count_is_half_the_square_root_between_2_and_8() {
        let counts = [6, 10, 70, 100, 217, 1000].map(group_count);
        assert_eq!(counts, [2, 2, 4, 5, 7, 8]);
    }

    #[test]
    fn fewer_than_six_played_games_make_no_families() {
        let five = library(
            &[
                (1, 10.0, PUZZLE),
                (2, 10.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 10.0, FIGHTING),
                (5, 10.0, FIGHTING),
            ],
            &[],
        );
        assert!(families_of(&five, &Corrections::default()).is_empty());

        let six = library(
            &[
                (1, 10.0, PUZZLE),
                (2, 10.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 10.0, FIGHTING),
                (5, 10.0, FIGHTING),
                (6, 10.0, FIGHTING),
            ],
            &[],
        );
        assert_eq!(families_of(&six, &Corrections::default()).len(), 2);
    }

    #[test]
    fn two_distant_genres_make_two_families_the_heavier_first() {
        let library = library(
            &[
                (1, 5.0, PUZZLE),
                (2, 5.0, PUZZLE),
                (3, 5.0, PUZZLE),
                (4, 5.0, PUZZLE),
                (5, 5.0, PUZZLE),
                (6, 50.0, FIGHTING),
                (7, 50.0, FIGHTING),
                (8, 50.0, FIGHTING),
                (9, 50.0, FIGHTING),
                (10, 50.0, FIGHTING),
            ],
            &[],
        );

        let families = families_of(&library, &Corrections::default());

        assert_eq!(families.len(), 2);
        assert_eq!(sorted(&families[0].games), ids(&[6, 7, 8, 9, 10]));
        assert_eq!(sorted(&families[1].games), ids(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn the_same_library_gives_the_same_families_in_any_order() {
        let library = library(
            &[
                (1, 30.0, PUZZLE),
                (2, 12.0, &[(1, 1.0), (3, 0.4)]),
                (3, 8.0, PUZZLE),
                (4, 90.0, FIGHTING),
                (5, 20.0, &[(3, 1.0), (5, 0.6)]),
                (6, 4.0, FIGHTING),
                (7, 60.0, FARMING),
                (8, 15.0, &[(5, 1.0), (1, 0.7)]),
                (9, 3.0, FARMING),
                (10, 7.0, &[(2, 1.0), (6, 1.0)]),
            ],
            &[],
        );
        let taste = taste(&library, &Corrections::default());
        let mut reversed = taste.clone();
        reversed.games.reverse();

        let first = automatic_families(&taste, &library.signals, &library.catalog);
        let again = automatic_families(&taste, &library.signals, &library.catalog);
        let from_reversed = automatic_families(&reversed, &library.signals, &library.catalog);

        assert!(!first.is_empty());
        assert_eq!(first, again);
        assert_eq!(first, from_reversed);
    }

    #[test]
    fn a_group_of_two_games_is_not_a_family() {
        let library = library(
            &[
                (1, 10.0, PUZZLE),
                (2, 10.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 10.0, PUZZLE),
                (5, 10.0, PUZZLE),
                (6, 10.0, PUZZLE),
                (7, 10.0, PUZZLE),
                (8, 10.0, PUZZLE),
                (9, 10.0, FIGHTING),
                (10, 10.0, FIGHTING),
            ],
            &[],
        );

        let families = families_of(&library, &Corrections::default());

        assert_eq!(families.len(), 1);
        assert_eq!(families[0].games, ids(&[1, 2, 3, 4, 5, 6, 7, 8]));
    }

    #[test]
    fn a_family_is_the_eight_games_with_most_hours_in_order_of_hours() {
        let library = library(
            &[
                (1, 3.0, PUZZLE),
                (2, 250.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 300.0, PUZZLE),
                (5, 9.0, PUZZLE),
                (6, 8.0, PUZZLE),
                (7, 7.0, PUZZLE),
                (8, 7.0, PUZZLE),
                (9, 5.0, PUZZLE),
                (10, 4.0, PUZZLE),
                (11, 10.0, FIGHTING),
                (12, 10.0, FIGHTING),
                (13, 10.0, FIGHTING),
            ],
            &[],
        );

        let families = families_of(&library, &Corrections::default());

        assert_eq!(families[0].games, ids(&[4, 2, 3, 5, 6, 7, 8, 9]));
    }

    #[test]
    fn a_game_off_the_catalog_is_in_no_family() {
        let mut library = library(
            &[
                (1, 10.0, PUZZLE),
                (2, 10.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 10.0, FIGHTING),
                (5, 10.0, FIGHTING),
                (6, 10.0, FIGHTING),
            ],
            &[],
        );
        library.signals.library.push(LibraryGame {
            id: AppId(7),
            total_hours: 500.0,
            last_played: None,
        });
        let off_catalog = BTreeMap::from([(
            AppId(7),
            OffCatalogGame {
                tags: vec![GameTag {
                    id: TagId(1),
                    weight: 1.0,
                }],
                name: None,
            },
        )]);
        let config = SignalsConfig::default();
        let hours = hours_by_source(&library.signals, &Votes::new(), &BTreeSet::new(), &config);
        let taste = signals_taste(
            &hours,
            &library.catalog,
            &off_catalog,
            &Corrections::default(),
            &config,
        );
        assert!(taste.games.iter().any(|game| game.id == AppId(7)));

        let families = automatic_families(&taste, &library.signals, &library.catalog);

        assert_eq!(families.len(), 2);
        assert!(
            families
                .iter()
                .all(|family| !family.games.contains(&AppId(7)))
        );
    }

    #[test]
    fn a_wishlist_game_is_in_no_family() {
        let library = library(
            &[
                (1, 10.0, PUZZLE),
                (2, 10.0, PUZZLE),
                (3, 10.0, PUZZLE),
                (4, 10.0, FIGHTING),
                (5, 10.0, FIGHTING),
                (6, 10.0, FIGHTING),
            ],
            &[(7, PUZZLE)],
        );
        assert!(
            taste(&library, &Corrections::default())
                .games
                .iter()
                .any(|game| game.id == AppId(7))
        );

        let families = families_of(&library, &Corrections::default());

        assert!(
            families
                .iter()
                .all(|family| !family.games.contains(&AppId(7)))
        );
    }

    #[test]
    fn a_banned_tag_does_not_decide_the_group() {
        let library = library(
            &[
                (1, 10.0, &[(1, 0.1), (2, 0.1)]),
                (2, 10.0, &[(1, 0.1), (2, 0.1)]),
                (3, 10.0, &[(1, 0.1), (2, 0.1)]),
                (4, 10.0, &[(1, 0.1), (2, 0.1), (9, 1.0)]),
                (5, 10.0, &[(3, 0.1), (4, 0.1), (9, 1.0)]),
                (6, 10.0, &[(3, 0.1), (4, 0.1), (9, 1.0)]),
                (7, 10.0, &[(3, 0.1), (4, 0.1), (9, 1.0)]),
            ],
            &[],
        );
        let family_of = |families: &[Family], game: u32| {
            families
                .iter()
                .position(|family| family.games.contains(&AppId(game)))
        };

        let unbanned = families_of(&library, &Corrections::default());
        assert_eq!(family_of(&unbanned, 4), family_of(&unbanned, 5));

        let banned = families_of(
            &library,
            &Corrections {
                tag_multipliers: BTreeMap::from([(TagId(9), 0.0)]),
                ..Corrections::default()
            },
        );
        assert_eq!(family_of(&banned, 4), family_of(&banned, 1));
        assert_ne!(family_of(&banned, 4), family_of(&banned, 5));
        assert!(banned.iter().all(|family| !family.name.contains("tag 9")));
    }

    #[test]
    fn the_name_uses_the_tags_that_set_the_group_apart() {
        const ROGUELIKE: Tags = &[(1, 1.0), (2, 0.6), (3, 0.5), (4, 0.4)];
        const SHOOTER: Tags = &[(1, 1.0), (5, 0.6), (6, 0.5), (7, 0.4)];
        let library = library(
            &[
                (1, 20.0, ROGUELIKE),
                (2, 20.0, ROGUELIKE),
                (3, 20.0, ROGUELIKE),
                (4, 10.0, SHOOTER),
                (5, 10.0, SHOOTER),
                (6, 10.0, SHOOTER),
            ],
            &[],
        );

        let names: Vec<String> = families_of(&library, &Corrections::default())
            .into_iter()
            .map(|family| family.name)
            .collect();

        assert_eq!(names, ["tag 2 · tag 3 · tag 4", "tag 5 · tag 6 · tag 7"]);
    }

    #[test]
    fn a_single_group_is_named_after_its_heaviest_tags() {
        let library = library(
            &[
                (1, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
                (2, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
                (3, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
                (4, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
                (5, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
                (6, 10.0, &[(1, 0.2), (2, 1.0), (3, 0.5), (4, 0.7)]),
            ],
            &[],
        );

        let families = families_of(&library, &Corrections::default());

        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name, "tag 2 · tag 4 · tag 3");
        assert_eq!(families[0].games, ids(&[1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn a_repeated_name_gets_a_number() {
        let names = ["Puzzle", "Puzzle", "Fighting", "Puzzle"].map(String::from);
        assert_eq!(
            unique_names(names.to_vec()),
            ["Puzzle", "Puzzle 2", "Fighting", "Puzzle 3"]
        );
    }
}
