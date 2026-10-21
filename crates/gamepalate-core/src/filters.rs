use std::collections::BTreeSet;

use crate::{
    catalog::Catalog,
    config::Config,
    corrections::{Ban, Corrections},
    families::Families,
    game::{Game, Price},
    ids::{AppId, TagId},
    signals::Signals,
    timestamp::Timestamp,
    votes::{VoteKind, Votes},
};

#[derive(Debug, Clone, Default)]
pub struct FilterOptions {
    pub min_reviews: Option<u32>,
    pub max_reviews: Option<u32>,
    pub min_positive_percent: Option<u8>,
    pub without_voted: bool,
    pub without_early_access: Option<bool>,
    pub min_price_cents: Option<u32>,
    pub max_price_cents: Option<u32>,
    pub only_discounted: bool,
    pub min_discount_percent: Option<u8>,
    pub tags: Vec<String>,
    pub without_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    Owned,
    FamilyGames,
    Hidden,
    NotForMe,
    Known,
    BannedTags,
    MinReviews(u32),
    MaxReviews(u32),
    MinPositivePercent(u8),
    Voted,
    EarlyAccess,
    MinPrice(u32),
    MaxPrice(u32),
    OnlyDiscounted,
    MinDiscount(u8),
    RequiredTags(Vec<TagId>),
    ExcludedTags(Vec<TagId>),
}

impl Filter {
    pub fn name(&self) -> &'static str {
        match self {
            Filter::Owned => "owned games",
            Filter::FamilyGames => "family games",
            Filter::Hidden => "hidden games",
            Filter::NotForMe => "not for me games",
            Filter::Known => "known games",
            Filter::BannedTags => "banned tags",
            Filter::MinReviews(_) => "min reviews",
            Filter::MaxReviews(_) => "max reviews",
            Filter::MinPositivePercent(_) => "min positive percent",
            Filter::Voted => "voted games",
            Filter::EarlyAccess => "early access",
            Filter::MinPrice(_) => "min price",
            Filter::MaxPrice(_) => "max price",
            Filter::OnlyDiscounted => "only discounted",
            Filter::MinDiscount(_) => "min discount",
            Filter::RequiredTags(_) => "required tags",
            Filter::ExcludedTags(_) => "excluded tags",
        }
    }

    pub fn is_blamable(&self) -> bool {
        !matches!(self, Filter::Owned | Filter::FamilyGames)
    }

    pub fn passes(&self, game: &Game, price: Price, context: &FilterContext) -> bool {
        match self {
            Filter::Owned => !context.owned.contains(&game.id),
            Filter::FamilyGames => !context.family_games.contains(&game.id),
            Filter::Hidden => !context.corrections.hidden_games.contains(&game.id),
            Filter::NotForMe => context.vote_kind(game.id) != Some(VoteKind::NotForMe),
            Filter::Known => context.vote_kind(game.id) != Some(VoteKind::Known),
            Filter::BannedTags => !context
                .corrections
                .banned_tags
                .iter()
                .any(|(&tag_id, &ban)| hits_ban(game, tag_id, ban)),
            Filter::MinReviews(min) => game.reviews.total >= *min,
            Filter::MaxReviews(max) => game.reviews.total <= *max,
            Filter::MinPositivePercent(min) => {
                game.reviews.total > 0
                    && u64::from(game.reviews.positive) * 100
                        >= u64::from(*min) * u64::from(game.reviews.total)
            }
            Filter::Voted => context.vote_kind(game.id).is_none(),
            Filter::EarlyAccess => !game.is_early_access,
            Filter::MinPrice(min) => price.final_cents() >= *min,
            Filter::MaxPrice(max) => price.final_cents() <= *max,
            Filter::OnlyDiscounted => discount_percent(price) > 0,
            Filter::MinDiscount(min) => {
                let percent = discount_percent(price);
                percent > 0 && percent >= *min
            }
            Filter::RequiredTags(tag_ids) => tag_ids.iter().all(|&id| has_tag(game, id)),
            Filter::ExcludedTags(tag_ids) => !tag_ids.iter().any(|&id| has_tag(game, id)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterContext<'a> {
    pub owned: BTreeSet<AppId>,
    pub family_games: BTreeSet<AppId>,
    pub votes: &'a Votes,
    pub corrections: &'a Corrections,
}

impl<'a> FilterContext<'a> {
    pub fn new(
        signals: &Signals,
        families: &Families,
        votes: &'a Votes,
        corrections: &'a Corrections,
    ) -> FilterContext<'a> {
        let owned = signals.library.iter().map(|game| game.id).collect();
        let family_games = families
            .families
            .iter()
            .flat_map(|family| family.games.iter().copied())
            .collect();

        FilterContext {
            owned,
            family_games,
            votes,
            corrections,
        }
    }

    fn vote_kind(&self, id: AppId) -> Option<VoteKind> {
        self.votes.get(&id).map(|vote| vote.kind)
    }
}

#[derive(Debug, Clone)]
pub struct Blame {
    pub filter: &'static str,
    pub discarded: usize,
}

#[derive(Debug, Clone)]
pub struct FilterOutcome {
    pub candidates: BTreeSet<AppId>,
    pub blame: Option<Blame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagNameError {
    NotFound(String),
    Ambiguous(String),
}

fn has_tag(game: &Game, tag_id: TagId) -> bool {
    game.tags.iter().any(|tag| tag.id == tag_id)
}

fn discount_percent(price: Price) -> u8 {
    price.discount.map_or(0, |discount| discount.percent)
}

pub fn hits_ban(game: &Game, tag_id: TagId, ban: Ban) -> bool {
    let Some(banned) = game.tags.iter().find(|tag| tag.id == tag_id) else {
        return false;
    };

    match ban {
        Ban::Everywhere => true,
        Ban::TopTags(limit) => {
            let heavier = game
                .tags
                .iter()
                .filter(|tag| tag.weight > banned.weight)
                .count();
            heavier < limit as usize
        }
    }
}

fn normalize_tag_name(name: &str) -> String {
    name.trim().to_lowercase()
}

pub fn find_tag(name: &str, catalog: &Catalog) -> Result<TagId, TagNameError> {
    let wanted = normalize_tag_name(name);
    let mut matches = catalog
        .tags
        .iter()
        .filter(|(_, tag)| {
            normalize_tag_name(&tag.english) == wanted || normalize_tag_name(&tag.italian) == wanted
        })
        .map(|(&id, _)| id);

    match (matches.next(), matches.next()) {
        (Some(id), None) => Ok(id),
        (None, _) => Err(TagNameError::NotFound(name.to_string())),
        (Some(_), Some(_)) => Err(TagNameError::Ambiguous(name.to_string())),
    }
}

fn find_tags(names: &[String], catalog: &Catalog) -> Result<Vec<TagId>, TagNameError> {
    names.iter().map(|name| find_tag(name, catalog)).collect()
}

pub fn active_filters(
    options: &FilterOptions,
    config: &Config,
    catalog: &Catalog,
) -> Result<Vec<Filter>, TagNameError> {
    let mut filters = vec![
        Filter::Owned,
        Filter::FamilyGames,
        Filter::Hidden,
        Filter::NotForMe,
        Filter::Known,
        Filter::BannedTags,
        Filter::MinReviews(options.min_reviews.unwrap_or(config.relevance.min_reviews)),
    ];

    if let Some(max) = options.max_reviews {
        filters.push(Filter::MaxReviews(max));
    }
    let min_positive_percent = options
        .min_positive_percent
        .unwrap_or(config.results.min_positive_percent);
    if min_positive_percent != 0 {
        filters.push(Filter::MinPositivePercent(min_positive_percent));
    }
    if options.without_voted {
        filters.push(Filter::Voted);
    }
    if options
        .without_early_access
        .unwrap_or(!config.results.early_access)
    {
        filters.push(Filter::EarlyAccess);
    }
    if let Some(min) = options.min_price_cents {
        filters.push(Filter::MinPrice(min));
    }
    if let Some(max) = options.max_price_cents {
        filters.push(Filter::MaxPrice(max));
    }
    match options.min_discount_percent {
        Some(min) => filters.push(Filter::MinDiscount(min)),
        None if options.only_discounted => filters.push(Filter::OnlyDiscounted),
        None => {}
    }
    if !options.tags.is_empty() {
        filters.push(Filter::RequiredTags(find_tags(&options.tags, catalog)?));
    }
    if !options.without_tags.is_empty() {
        filters.push(Filter::ExcludedTags(find_tags(
            &options.without_tags,
            catalog,
        )?));
    }

    Ok(filters)
}

pub fn apply_filters(
    catalog: &Catalog,
    filters: &[Filter],
    context: &FilterContext,
    now: Timestamp,
) -> FilterOutcome {
    let mut candidates = BTreeSet::new();
    // (filter index, discarded games), in the order of each filter's first discard
    let mut discards: Vec<(usize, usize)> = Vec::new();

    for game in catalog.games() {
        let price = game.price.as_of(now);
        match filters
            .iter()
            .position(|filter| !filter.passes(game, price, context))
        {
            None => {
                candidates.insert(game.id);
            }
            Some(index) if filters[index].is_blamable() => {
                match discards.iter_mut().find(|(seen, _)| *seen == index) {
                    Some((_, count)) => *count += 1,
                    None => discards.push((index, 1)),
                }
            }
            Some(_) => {}
        }
    }

    let mut blamed: Option<(usize, usize)> = None;
    for &(index, count) in &discards {
        if blamed.is_none_or(|(_, best)| count > best) {
            blamed = Some((index, count));
        }
    }

    FilterOutcome {
        candidates,
        blame: blamed.map(|(index, discarded)| Blame {
            filter: filters[index].name(),
            discarded,
        }),
    }
}
