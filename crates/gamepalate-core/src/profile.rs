use std::collections::BTreeMap;

use crate::{
    corrections::{Ban, Corrections},
    effective_hours::effective_hours,
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
    }
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
    let hours = effective_hours(inputs.signals, inputs.votes, &inputs.config.signals);
    let taste = signals_taste(
        &hours,
        inputs.catalog,
        inputs.off_catalog,
        inputs.corrections,
        &inputs.config.signals,
    );
    let profile = taste.profile();
    if profile.is_empty() {
        return None;
    }
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

    let family_problems = inputs.families.problems(inputs.catalog, inputs.off_catalog);

    Some(Profile {
        tags,
        corrected_tags: corrected.into_iter().map(describe).collect(),
        corrected_games: corrected_games(&hours, inputs),
        families: family_profiles(inputs, &family_problems),
        family_problems,
        kept_aside: inputs.families.held_out.len(),
        saved_signals_from: inputs.signals.read_at,
    })
}
