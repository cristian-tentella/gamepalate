use std::collections::BTreeMap;

use crate::{
    corrections::{Ban, Corrections},
    effective_hours::{HoursSource, hours_by_source},
    families::FamilyProblem,
    ids::{AppId, TagId},
    search::SearchInputs,
    signals::Signals,
    tag_vector::{TagVector, add_weighted, tf_idf_vector},
    taste::{Taste, family_taste, game_tags, signals_taste},
    timestamp::Timestamp,
    votes::{VoteKind, Votes},
};

pub const DEFAULT_TAG_LIMIT: usize = 15;
const TAG_GAMES: usize = 3;
const FAMILY_TAGS: usize = 10;
const DOMINANT_RATIO: f64 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Library,
    Recent,
    Wishlist,
    Followed,
    WouldTry,
}

#[derive(Debug, Clone)]
pub struct TagCorrection {
    pub banned: Option<Ban>,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct TagContribution {
    pub id: AppId,
    pub share: f64,
    pub kinds: Vec<SignalKind>,
}

#[derive(Debug, Clone)]
pub struct ProfileTag {
    pub id: TagId,
    pub weight: f64,
    pub weight_before: f64,
    pub correction: Option<TagCorrection>,
    pub games: Vec<TagContribution>,
    pub game_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileGame {
    pub id: AppId,
    pub weight: f64,
    pub hours: f64,
    pub source: HoursSource,
}

#[derive(Debug, Clone)]
pub struct CorrectedGame {
    pub id: AppId,
    pub multiplier: f64,
    pub in_effect: bool,
    pub weight_before: f64,
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct FamilyProfile {
    pub name: String,
    pub games: Vec<AppId>,
    pub tags: Vec<(TagId, f64)>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub tags: Vec<ProfileTag>,
    pub games: Vec<ProfileGame>,
    pub recent_games: usize,
    pub corrected_tags: Vec<ProfileTag>,
    pub corrected_games: Vec<CorrectedGame>,
    pub families: Vec<FamilyProfile>,
    pub family_problems: Vec<FamilyProblem>,
    pub kept_aside: usize,
    pub saved_signals_from: Timestamp,
}

// Where the signals profile comes from: the weight of every game on every tag, before tag multipliers.
#[derive(Debug, Clone)]
pub struct Contributions {
    pub before: TagVector,
    pub by_tag: BTreeMap<TagId, Vec<(AppId, f64)>>,
}

impl Profile {
    pub fn source_shares(&self) -> Vec<(HoursSource, f64)> {
        let total: f64 = self.games.iter().map(|game| game.weight).sum();
        [
            HoursSource::Played,
            HoursSource::Followed,
            HoursSource::Wishlist,
            HoursSource::WouldTry,
        ]
        .into_iter()
        .map(|source| {
            let weight: f64 = self
                .games
                .iter()
                .filter(|game| game.source == source)
                .map(|game| game.weight)
                .sum();
            (source, if total == 0.0 { 0.0 } else { weight / total })
        })
        .collect()
    }
}

pub fn contributions(taste: &Taste, inputs: &SearchInputs) -> Contributions {
    let mut before = TagVector::new();
    let mut by_tag: BTreeMap<TagId, Vec<(AppId, f64)>> = BTreeMap::new();

    for game in &taste.games {
        let tags = game_tags(game.id, inputs.catalog, inputs.off_catalog).unwrap_or_default();
        let tf_idf_vector = tf_idf_vector(tags, &inputs.catalog.idf);
        add_weighted(&mut before, &tf_idf_vector, game.weight);

        for (&tag, &value) in &tf_idf_vector {
            by_tag
                .entry(tag)
                .or_default()
                .push((game.id, game.weight * value));
        }
    }

    Contributions { before, by_tag }
}

pub fn signal_kinds(id: AppId, signals: &Signals, votes: &Votes) -> Vec<SignalKind> {
    let would_try = votes
        .get(&id)
        .is_some_and(|vote| vote.kind == VoteKind::WouldTry);

    [
        (
            SignalKind::Library,
            signals.library.iter().any(|game| game.id == id),
        ),
        (
            SignalKind::Recent,
            signals.recent.iter().any(|game| game.id == id),
        ),
        (SignalKind::Wishlist, signals.wishlist.contains(&id)),
        (SignalKind::Followed, signals.followed.contains(&id)),
        (SignalKind::WouldTry, would_try),
    ]
    .into_iter()
    .filter(|&(_, is_kind)| is_kind)
    .map(|(kind, _)| kind)
    .collect()
}

fn tag_correction(tag: TagId, corrections: &Corrections) -> Option<TagCorrection> {
    let banned = corrections.banned_tags.get(&tag).copied();
    let multiplier = corrections.tag_multipliers.get(&tag).copied();
    if banned.is_none() && multiplier.is_none() {
        return None;
    }

    Some(TagCorrection { banned, multiplier })
}

pub fn describe_tag(
    tag: TagId,
    profile: &TagVector,
    contributions: &Contributions,
    inputs: &SearchInputs,
) -> ProfileTag {
    let weight_before = contributions.before.get(&tag).copied().unwrap_or(0.0);

    let mut games = contributions.by_tag.get(&tag).cloned().unwrap_or_default();
    let game_count = games.len();
    games.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let games = games
        .into_iter()
        .take(TAG_GAMES)
        .map(|(id, value)| TagContribution {
            id,
            share: if weight_before == 0.0 {
                0.0
            } else {
                value / weight_before
            },
            kinds: signal_kinds(id, inputs.signals, inputs.votes),
        })
        .collect();

    ProfileTag {
        id: tag,
        weight: profile.get(&tag).copied().unwrap_or(0.0),
        weight_before,
        correction: tag_correction(tag, inputs.corrections),
        games,
        game_count,
    }
}

pub fn dominant_game(tag: &ProfileTag) -> Option<AppId> {
    let first = tag.games.first()?;
    let second = tag.games.get(1).map_or(0.0, |game| game.share);
    (first.share > 0.0 && first.share >= DOMINANT_RATIO * second).then_some(first.id)
}

pub fn profile_games(
    taste: &Taste,
    hours: &BTreeMap<AppId, (f64, HoursSource)>,
) -> Vec<ProfileGame> {
    let mut games: Vec<ProfileGame> = taste
        .games
        .iter()
        .filter_map(|game| {
            let &(hours, source) = hours.get(&game.id)?;
            Some(ProfileGame {
                id: game.id,
                weight: game.weight * game.tag_vector.values().sum::<f64>(),
                hours,
                source,
            })
        })
        .collect();
    games.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
    games
}

// Heaviest first, ties by tag id.
pub fn heaviest_tags(profile: &TagVector) -> Vec<(TagId, f64)> {
    let mut tags: Vec<(TagId, f64)> = profile.iter().map(|(&id, &value)| (id, value)).collect();
    tags.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    tags
}

pub fn corrected_games(hours: &BTreeMap<AppId, f64>, inputs: &SearchInputs) -> Vec<CorrectedGame> {
    let signals_config = &inputs.config.signals;

    inputs
        .corrections
        .game_multipliers
        .iter()
        .map(|(&id, &multiplier)| {
            let game_hours = hours
                .get(&id)
                .copied()
                .filter(|&game_hours| game_hours >= signals_config.min_hours);
            let weight_before = game_hours.map_or(0.0, |game_hours| {
                f64::min(game_hours, signals_config.max_hours).ln_1p()
            });

            CorrectedGame {
                id,
                multiplier,
                in_effect: game_hours.is_some(),
                weight_before,
                weight: weight_before * multiplier,
            }
        })
        .collect()
}

pub fn family_profiles(inputs: &SearchInputs, problems: &[FamilyProblem]) -> Vec<FamilyProfile> {
    inputs
        .families
        .clone()
        .without_problems(problems)
        .families
        .into_iter()
        .map(|family| {
            let taste = family_taste(
                &family,
                inputs.catalog,
                inputs.off_catalog,
                &inputs.corrections.tag_multipliers,
            );
            let tags = heaviest_tags(&taste.profile())
                .into_iter()
                .filter(|&(_, weight)| weight > 0.0)
                .take(FAMILY_TAGS)
                .collect();

            FamilyProfile {
                name: family.name,
                games: family.games,
                tags,
            }
        })
        .collect()
}

// None when no game is above the hours threshold.
pub fn profile(inputs: &SearchInputs, limit: usize) -> Option<Profile> {
    let hours_by_source = hours_by_source(inputs.signals, inputs.votes, &inputs.config.signals);
    let hours: BTreeMap<AppId, f64> = hours_by_source
        .iter()
        .map(|(&id, &(hours, _))| (id, hours))
        .collect();
    let taste = signals_taste(
        &hours,
        inputs.catalog,
        inputs.off_catalog,
        inputs.corrections,
        &inputs.config.signals,
    );
    if !taste.has_profile() {
        return None;
    }
    let profile = taste.profile();
    let contributions = contributions(&taste, inputs);
    let describe = |tag: TagId| describe_tag(tag, &profile, &contributions, inputs);

    let tags = heaviest_tags(&profile)
        .into_iter()
        .take(limit)
        .map(|(tag, _)| describe(tag))
        .collect();

    let corrections = inputs.corrections;
    let mut corrected: Vec<TagId> = corrections.banned_tags.keys().copied().collect();
    for &tag in corrections.tag_multipliers.keys() {
        if !corrected.contains(&tag) {
            corrected.push(tag);
        }
    }

    let family_problems = inputs.families.problems(
        inputs.catalog,
        inputs.off_catalog,
        &corrections.tag_multipliers,
    );

    let recent_games = taste
        .games
        .iter()
        .filter(|game| {
            inputs
                .signals
                .recent
                .iter()
                .any(|recent| recent.id == game.id)
        })
        .count();

    Some(Profile {
        tags,
        games: profile_games(&taste, &hours_by_source),
        recent_games,
        corrected_tags: corrected.into_iter().map(describe).collect(),
        corrected_games: corrected_games(&hours, inputs),
        families: family_profiles(inputs, &family_problems),
        family_problems,
        kept_aside: inputs.families.held_out.len(),
        saved_signals_from: inputs.signals.read_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{Catalog, Freshness},
        config::Config,
        currency::Currency,
        families::Families,
        game::{Game, GameTag, Price, Reviews},
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, RecentGame},
        votes::Vote,
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
        }
    }

    fn test_profile() -> Profile {
        let catalog = Catalog::new(
            vec![
                game(1, &[1]),
                game(2, &[1]),
                game(3, &[1, 2]),
                game(4, &[2]),
                game(5, &[1]),
            ],
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 1.0), (TagId(2), 1.0)]),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![
                LibraryGame {
                    id: AppId(1),
                    total_hours: 20.0,
                },
                LibraryGame {
                    id: AppId(2),
                    total_hours: 1.0,
                },
            ],
            recent: vec![RecentGame {
                id: AppId(5),
                total_hours: 2.0,
                recent_hours: 1.0,
            }],
            wishlist: vec![AppId(2)],
            followed: vec![AppId(3)],
        };
        let votes = Votes::from([(
            AppId(4),
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-01".to_string(),
                bet: false,
            },
        )]);
        let inputs = SearchInputs {
            catalog: &catalog,
            game_vectors: &[],
            signals: &signals,
            votes: &votes,
            corrections: &Corrections::default(),
            families: &Families::default(),
            off_catalog: &OffCatalogGames::new(),
            config: &Config::default(),
        };

        profile(&inputs, DEFAULT_TAG_LIMIT).unwrap()
    }

    fn rounded(value: f64) -> f64 {
        (value * 1000.0).round() / 1000.0
    }

    #[test]
    fn the_games_come_heaviest_first_with_the_source_of_their_hours() {
        let games: Vec<(AppId, f64, HoursSource)> = test_profile()
            .games
            .iter()
            .map(|game| (game.id, game.hours, game.source))
            .collect();

        assert_eq!(
            games,
            [
                (AppId(1), 20.0, HoursSource::Played),
                (AppId(3), 3.0, HoursSource::Followed),
                (AppId(2), 5.0, HoursSource::Wishlist),
                (AppId(4), 5.0, HoursSource::WouldTry),
                (AppId(5), 4.0, HoursSource::Played),
            ]
        );
    }

    #[test]
    fn the_weight_of_a_game_sums_all_its_tags() {
        let profile = test_profile();

        assert_eq!(rounded(profile.games[0].weight), rounded(21.0f64.ln()));
        assert_eq!(rounded(profile.games[1].weight), rounded(2.0 * 4.0f64.ln()));
    }

    #[test]
    fn the_source_shares_split_the_weight_of_the_games() {
        let played = 21.0f64.ln() + 5.0f64.ln();
        let followed = 2.0 * 4.0f64.ln();
        let wishlist = 6.0f64.ln();
        let total = played + followed + 2.0 * wishlist;

        let shares: Vec<(HoursSource, f64)> = test_profile()
            .source_shares()
            .into_iter()
            .map(|(source, share)| (source, rounded(share)))
            .collect();

        assert_eq!(
            shares,
            [
                (HoursSource::Played, rounded(played / total)),
                (HoursSource::Followed, rounded(followed / total)),
                (HoursSource::Wishlist, rounded(wishlist / total)),
                (HoursSource::WouldTry, rounded(wishlist / total)),
            ]
        );
    }

    #[test]
    fn every_tag_counts_the_games_that_carry_it() {
        let counts: Vec<(TagId, usize)> = test_profile()
            .tags
            .iter()
            .map(|tag| (tag.id, tag.game_count))
            .collect();

        assert_eq!(counts, [(TagId(1), 4), (TagId(2), 2)]);
    }

    #[test]
    fn counts_the_recent_games_of_the_taste() {
        assert_eq!(test_profile().recent_games, 1);
    }

    fn tag_with_shares(shares: &[f64]) -> ProfileTag {
        ProfileTag {
            id: TagId(1),
            weight: 1.0,
            weight_before: 1.0,
            correction: None,
            games: shares
                .iter()
                .enumerate()
                .map(|(i, &share)| TagContribution {
                    id: AppId(i as u32 + 1),
                    share,
                    kinds: vec![SignalKind::Library],
                })
                .collect(),
            game_count: shares.len(),
        }
    }

    #[test]
    fn a_game_far_ahead_of_the_next_dominates_its_tag() {
        assert_eq!(
            dominant_game(&tag_with_shares(&[0.1136, 0.0147, 0.0125])),
            Some(AppId(1))
        );
        assert_eq!(
            dominant_game(&tag_with_shares(&[0.0727, 0.0237, 0.0221])),
            Some(AppId(1))
        );
    }

    #[test]
    fn a_game_twice_the_next_does_not_dominate() {
        assert_eq!(
            dominant_game(&tag_with_shares(&[0.0590, 0.0299, 0.0254])),
            None
        );
    }

    #[test]
    fn a_tag_with_one_game_is_dominated_by_it() {
        assert_eq!(dominant_game(&tag_with_shares(&[1.0])), Some(AppId(1)));
        assert_eq!(dominant_game(&tag_with_shares(&[])), None);
    }
}
