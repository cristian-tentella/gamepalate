use std::collections::BTreeSet;

use crate::{
    catalog::Catalog,
    config::{Config, SignalsConfig},
    corrections::{Ban, Corrections},
    effective_hours::effective_hours,
    families::Families,
    game::{Game, Price},
    ids::{AppId, TagId},
    signals::Signals,
    timestamp::Timestamp,
    votes::{VoteKind, Votes, standing},
};

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
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

/// Where the candidates of a search come from: the games you don't know, or the ones you own
/// and haven't played yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverMode {
    #[default]
    New,
    Backlog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    Owned,
    Backlog,
    WishedOrFollowed,
    FamilyGames,
    Hidden,
    NotForMe,
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
            Filter::Backlog => "games outside the backlog",
            Filter::WishedOrFollowed => "wishlist and followed games",
            Filter::FamilyGames => "family games",
            Filter::Hidden => "hidden games",
            Filter::NotForMe => "not for me games",
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
        !matches!(
            self,
            Filter::Owned | Filter::Backlog | Filter::WishedOrFollowed | Filter::FamilyGames
        )
    }

    pub fn passes(
        &self,
        game: &Game,
        price: Price,
        context: &FilterContext,
        now: Timestamp,
    ) -> bool {
        match self {
            Filter::Owned => !context.owned.contains(&game.id),
            Filter::Backlog => context.backlog.contains(&game.id),
            Filter::WishedOrFollowed => !context.wished_or_followed.contains(&game.id),
            Filter::FamilyGames => !context.family_games.contains(&game.id),
            Filter::Hidden => !context.corrections.hidden_games.contains(&game.id),
            Filter::NotForMe => !context.vote_kind(game.id).is_some_and(VoteKind::is_no),
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
            Filter::Voted => standing(context.votes, game.id, now).is_none(),
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
    /// The games of the library played under `min_hours` effective hours.
    pub backlog: BTreeSet<AppId>,
    pub wished_or_followed: BTreeSet<AppId>,
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
        signals_config: &SignalsConfig,
    ) -> FilterContext<'a> {
        let owned = owned_games(signals, corrections);
        let backlog = backlog_games(signals, votes, corrections, signals_config);
        let wished_or_followed = signals
            .wishlist
            .iter()
            .chain(&signals.followed)
            .copied()
            .collect();
        let family_games = families
            .families
            .iter()
            .flat_map(|family| family.games.iter().copied())
            .collect();

        FilterContext {
            owned,
            backlog,
            wished_or_followed,
            family_games,
            votes,
            corrections,
        }
    }

    fn vote_kind(&self, id: AppId) -> Option<VoteKind> {
        self.votes.get(&id).map(|vote| vote.kind)
    }
}

/// The games in the library, borrowed, played recently or played elsewhere.
pub fn owned_games(signals: &Signals, corrections: &Corrections) -> BTreeSet<AppId> {
    signals
        .played()
        .chain(signals.recent.iter().map(|game| game.id))
        .chain(corrections.played_elsewhere.iter().copied())
        .collect()
}

/// The games of the library with fewer effective hours than `min_hours`, so no Signal. Borrowed
/// games are someone else's and stay out.
pub fn backlog_games(
    signals: &Signals,
    votes: &Votes,
    corrections: &Corrections,
    signals_config: &SignalsConfig,
) -> BTreeSet<AppId> {
    let hours = effective_hours(
        signals,
        votes,
        &corrections.played_elsewhere,
        signals_config,
    );
    signals
        .library
        .iter()
        .map(|game| game.id)
        .filter(|id| {
            hours
                .get(id)
                .is_some_and(|&hours| hours < signals_config.min_hours)
        })
        .collect()
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
    mode: DiscoverMode,
) -> Result<Vec<Filter>, TagNameError> {
    let backlog = mode == DiscoverMode::Backlog;
    // In the backlog the game is already bought: nothing about price or reviews counts
    let mut filters = if backlog {
        vec![
            Filter::Backlog,
            Filter::FamilyGames,
            Filter::Hidden,
            Filter::NotForMe,
            Filter::BannedTags,
        ]
    } else {
        vec![
            Filter::Owned,
            Filter::WishedOrFollowed,
            Filter::FamilyGames,
            Filter::Hidden,
            Filter::NotForMe,
            Filter::BannedTags,
            Filter::MinReviews(options.min_reviews.unwrap_or(config.relevance.min_reviews)),
        ]
    };

    if let Some(max) = options.max_reviews.filter(|_| !backlog) {
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
    if !backlog {
        filters.extend(price_filters(options));
    }
    filters.extend(tag_filters(options, catalog)?);

    Ok(filters)
}

fn price_and_tag_filters(
    options: &FilterOptions,
    catalog: &Catalog,
) -> Result<Vec<Filter>, TagNameError> {
    let mut filters = price_filters(options);
    filters.extend(tag_filters(options, catalog)?);
    Ok(filters)
}

fn price_filters(options: &FilterOptions) -> Vec<Filter> {
    let mut filters = Vec::new();
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
    filters
}

fn tag_filters(options: &FilterOptions, catalog: &Catalog) -> Result<Vec<Filter>, TagNameError> {
    let mut filters = Vec::new();
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

/// Only the filters the options ask for: no defaults from the config, and none of the
/// filters on owned, hidden or voted games. An empty option is no limit.
pub fn chosen_filters(
    options: &FilterOptions,
    catalog: &Catalog,
) -> Result<Vec<Filter>, TagNameError> {
    let mut filters = Vec::new();
    if let Some(min) = options.min_reviews {
        filters.push(Filter::MinReviews(min));
    }
    if let Some(max) = options.max_reviews {
        filters.push(Filter::MaxReviews(max));
    }
    if let Some(min) = options.min_positive_percent.filter(|&min| min != 0) {
        filters.push(Filter::MinPositivePercent(min));
    }
    if options.without_early_access == Some(true) {
        filters.push(Filter::EarlyAccess);
    }
    filters.extend(price_and_tag_filters(options, catalog)?);

    Ok(filters)
}

/// The first filter that leaves the game out, in the order of the filters.
pub fn first_failed<'f>(
    game: &Game,
    filters: &'f [Filter],
    context: &FilterContext,
    now: Timestamp,
) -> Option<&'f Filter> {
    let price = game.price.as_of(now);
    filters
        .iter()
        .find(|filter| !filter.passes(game, price, context, now))
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
            .position(|filter| !filter.passes(game, price, context, now))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{LibraryGame, RecentGame};

    #[test]
    fn owned_games_include_borrowed_and_recent_ones() {
        let game = |id| LibraryGame {
            id: AppId(id),
            total_hours: 3.0,
            last_played: None,
        };
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![game(1)],
            borrowed: vec![game(2)],
            recent: vec![RecentGame {
                id: AppId(3),
                total_hours: 1.0,
                recent_hours: 1.0,
            }],
            wishlist: vec![AppId(4)],
            followed: vec![],
        };
        let votes = Votes::new();
        let corrections = Corrections::default();

        let context = FilterContext::new(
            &signals,
            &Families::default(),
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );

        assert_eq!(
            context.owned,
            BTreeSet::from([AppId(1), AppId(2), AppId(3)])
        );
    }

    #[test]
    fn a_game_played_elsewhere_is_owned() {
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![],
            borrowed: vec![],
            recent: vec![],
            wishlist: vec![],
            followed: vec![],
        };
        let votes = Votes::new();
        let corrections = Corrections {
            played_elsewhere: BTreeSet::from([AppId(5)]),
            ..Corrections::default()
        };

        let context = FilterContext::new(
            &signals,
            &Families::default(),
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );

        assert_eq!(context.owned, BTreeSet::from([AppId(5)]));
    }

    fn catalog_with_tags() -> Catalog {
        use crate::{
            catalog::{Freshness, TagName},
            currency::Currency,
            region::Region,
        };
        use std::collections::BTreeMap;

        let tag = |name: &str| TagName {
            english: name.to_string(),
            italian: name.to_string(),
        };
        Catalog::new(
            Vec::new(),
            BTreeMap::from([(TagId(1), tag("Roguelike")), (TagId(2), tag("Horror"))]),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    #[test]
    fn no_chosen_option_is_no_filter() {
        let filters = chosen_filters(&FilterOptions::default(), &catalog_with_tags()).unwrap();

        assert_eq!(filters, Vec::new());
    }

    #[test]
    fn each_chosen_option_makes_its_filter() {
        let options = FilterOptions {
            min_reviews: Some(100),
            max_reviews: Some(5000),
            min_positive_percent: Some(85),
            without_voted: true,
            without_early_access: Some(true),
            min_price_cents: Some(500),
            max_price_cents: Some(2000),
            only_discounted: true,
            min_discount_percent: Some(30),
            tags: vec!["roguelike".to_string()],
            without_tags: vec!["Horror".to_string()],
        };

        let filters = chosen_filters(&options, &catalog_with_tags()).unwrap();

        assert_eq!(
            filters,
            vec![
                Filter::MinReviews(100),
                Filter::MaxReviews(5000),
                Filter::MinPositivePercent(85),
                Filter::EarlyAccess,
                Filter::MinPrice(500),
                Filter::MaxPrice(2000),
                Filter::MinDiscount(30),
                Filter::RequiredTags(vec![TagId(1)]),
                Filter::ExcludedTags(vec![TagId(2)]),
            ]
        );
    }

    #[test]
    fn only_discounted_alone_and_early_access_left_in_or_unset() {
        let only_discounted = FilterOptions {
            only_discounted: true,
            ..FilterOptions::default()
        };
        let catalog = catalog_with_tags();

        assert_eq!(
            chosen_filters(&only_discounted, &catalog).unwrap(),
            vec![Filter::OnlyDiscounted]
        );
        for early_access in [None, Some(false)] {
            let options = FilterOptions {
                without_early_access: early_access,
                ..FilterOptions::default()
            };
            assert_eq!(chosen_filters(&options, &catalog).unwrap(), Vec::new());
        }
    }

    #[test]
    fn a_zero_positive_percent_is_no_filter() {
        let options = FilterOptions {
            min_positive_percent: Some(0),
            ..FilterOptions::default()
        };

        assert_eq!(
            chosen_filters(&options, &catalog_with_tags()).unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn an_unknown_chosen_tag_is_an_error() {
        let options = FilterOptions {
            tags: vec!["Nope".to_string()],
            ..FilterOptions::default()
        };

        assert_eq!(
            chosen_filters(&options, &catalog_with_tags()),
            Err(TagNameError::NotFound("Nope".to_string()))
        );
    }

    #[test]
    fn the_active_filters_keep_their_order() {
        let options = FilterOptions {
            max_reviews: Some(5000),
            without_voted: true,
            min_price_cents: Some(500),
            max_price_cents: Some(2000),
            min_discount_percent: Some(30),
            tags: vec!["Roguelike".to_string()],
            without_tags: vec!["Horror".to_string()],
            ..FilterOptions::default()
        };
        let config = Config::default();

        let filters =
            active_filters(&options, &config, &catalog_with_tags(), DiscoverMode::New).unwrap();

        assert_eq!(
            filters,
            vec![
                Filter::Owned,
                Filter::WishedOrFollowed,
                Filter::FamilyGames,
                Filter::Hidden,
                Filter::NotForMe,
                Filter::BannedTags,
                Filter::MinReviews(config.relevance.min_reviews),
                Filter::MaxReviews(5000),
                Filter::MinPositivePercent(config.results.min_positive_percent),
                Filter::Voted,
                Filter::MinPrice(500),
                Filter::MaxPrice(2000),
                Filter::MinDiscount(30),
                Filter::RequiredTags(vec![TagId(1)]),
                Filter::ExcludedTags(vec![TagId(2)]),
            ]
        );
    }

    #[test]
    fn the_backlog_keeps_the_filters_about_what_you_want_to_play() {
        let options = FilterOptions {
            min_reviews: Some(500),
            max_reviews: Some(5000),
            min_positive_percent: Some(80),
            without_voted: true,
            without_early_access: Some(true),
            min_price_cents: Some(500),
            max_price_cents: Some(2000),
            only_discounted: true,
            min_discount_percent: Some(30),
            tags: vec!["Roguelike".to_string()],
            without_tags: vec!["Horror".to_string()],
        };

        let filters = active_filters(
            &options,
            &Config::default(),
            &catalog_with_tags(),
            DiscoverMode::Backlog,
        )
        .unwrap();

        assert_eq!(
            filters,
            vec![
                Filter::Backlog,
                Filter::FamilyGames,
                Filter::Hidden,
                Filter::NotForMe,
                Filter::BannedTags,
                Filter::MinPositivePercent(80),
                Filter::Voted,
                Filter::EarlyAccess,
                Filter::RequiredTags(vec![TagId(1)]),
                Filter::ExcludedTags(vec![TagId(2)]),
            ]
        );
    }

    #[test]
    fn the_backlog_ignores_the_minimum_reviews_of_the_config() {
        let filters = active_filters(
            &FilterOptions::default(),
            &Config::default(),
            &catalog_with_tags(),
            DiscoverMode::Backlog,
        )
        .unwrap();

        assert!(
            !filters
                .iter()
                .any(|filter| matches!(filter, Filter::MinReviews(_)))
        );
    }

    fn library_game(id: u32, hours: f64) -> crate::signals::LibraryGame {
        crate::signals::LibraryGame {
            id: AppId(id),
            total_hours: hours,
            last_played: None,
        }
    }

    #[test]
    fn the_backlog_is_the_library_under_the_minimum_effective_hours() {
        use crate::votes::Vote;

        let signals = Signals {
            read_at: Timestamp(0),
            // 1: untouched; 2: 5 h; 3: 0 h but played elsewhere; 4: 1 h with a Would try;
            // 5: 1 h but followed; 6: borrowed; 7: 1.5 h
            library: vec![
                library_game(1, 0.0),
                library_game(2, 5.0),
                library_game(3, 0.0),
                library_game(4, 1.0),
                library_game(5, 1.0),
                library_game(7, 1.5),
            ],
            borrowed: vec![library_game(6, 0.5)],
            recent: vec![],
            wishlist: vec![],
            followed: vec![AppId(5)],
        };
        let votes = Votes::from([(AppId(4), Vote::new(VoteKind::WouldTry, "2026-11-01"))]);
        let corrections = Corrections {
            played_elsewhere: BTreeSet::from([AppId(3)]),
            ..Corrections::default()
        };

        let context = FilterContext::new(
            &signals,
            &Families::default(),
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );

        assert_eq!(context.backlog, BTreeSet::from([AppId(1), AppId(7)]));
    }

    #[test]
    fn wishlist_and_followed_games_are_known() {
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![],
            borrowed: vec![],
            recent: vec![],
            wishlist: vec![AppId(1)],
            followed: vec![AppId(2)],
        };
        let votes = Votes::new();
        let corrections = Corrections::default();

        let context = FilterContext::new(
            &signals,
            &Families::default(),
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );

        assert_eq!(
            context.wished_or_followed,
            BTreeSet::from([AppId(1), AppId(2)])
        );
    }

    #[test]
    fn the_first_failed_filter_is_the_first_in_order() {
        let game = Game {
            id: AppId(7),
            name: "game 7".to_string(),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: crate::game::Reviews {
                total: 40,
                positive: 20,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        };
        let signals = Signals {
            read_at: Timestamp(0),
            library: vec![],
            borrowed: vec![],
            recent: vec![],
            wishlist: vec![],
            followed: vec![],
        };
        let votes = Votes::new();
        let corrections = Corrections::default();
        let context = FilterContext::new(
            &signals,
            &Families::default(),
            &votes,
            &corrections,
            &SignalsConfig::default(),
        );
        let filters = [
            Filter::Hidden,
            Filter::MinReviews(50),
            Filter::MinPositivePercent(70),
        ];

        assert_eq!(
            first_failed(&game, &filters, &context, Timestamp(0)),
            Some(&Filter::MinReviews(50))
        );
        assert_eq!(
            first_failed(&game, &filters[..1], &context, Timestamp(0)),
            None
        );
    }
}
