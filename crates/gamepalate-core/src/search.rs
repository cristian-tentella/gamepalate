use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use crate::{
    catalog::{Catalog, Freshness},
    config::Config,
    corrections::Corrections,
    creators::{LikedCreators, liked_creators},
    effective_hours::hours_by_source,
    explanation::{similar_to, top_tags},
    families::{Families, Family},
    filters::{Blame, FilterContext, FilterOptions, TagNameError, active_filters, apply_filters},
    game::{Price, Reviews},
    game_vector::GameVector,
    ids::AppId,
    off_catalog::OffCatalogGames,
    opinion::{opinion_model, opinions},
    quality::quality,
    ranking::{RankedGame, bets, ranking, with_votes},
    signals::Signals,
    taste::{Taste, family_taste, game_name, signals_taste},
    timestamp::Timestamp,
    turns::{ListKind, Pick, Turns, turn_shares, with_bets},
    votes::Votes,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SortBy {
    #[default]
    Relevance,
    Discount,
    Price,
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct SearchOptions {
    pub family: Option<String>,
    pub signals_only: bool,
    pub only_bets: bool,
    pub sort: SortBy,
    pub limit: Option<usize>,
    pub filters: FilterOptions,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchInputs<'a> {
    pub catalog: &'a Catalog,
    pub game_vectors: &'a [GameVector],
    pub signals: &'a Signals,
    pub votes: &'a Votes,
    pub corrections: &'a Corrections,
    pub families: &'a Families,
    pub off_catalog: &'a OffCatalogGames,
    pub config: &'a Config,
}

impl SearchInputs<'_> {
    pub fn liked_creators(&self) -> LikedCreators {
        liked_creators(
            self.signals,
            &self.corrections.played_elsewhere,
            &self.config.signals,
            self.catalog,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchError {
    FamilyNotFound { name: String, families: Vec<String> },
    TagName(TagNameError),
    EmptySignalsProfile,
}

impl From<TagNameError> for SearchError {
    fn from(error: TagNameError) -> SearchError {
        SearchError::TagName(error)
    }
}

#[derive(Debug, Clone)]
pub struct SearchTaste {
    pub family: Option<String>,
    pub kind: ListKind,
    pub taste: Taste,
}

#[derive(Debug, Clone)]
pub struct RecommendedGame {
    pub position: usize,
    pub id: AppId,
    pub name: String,
    pub price: Price,
    pub store_url: String,
    pub from_family: Option<String>,
    pub affinity: f64,
    pub top_tags: Vec<String>,
    pub similar_to: Option<String>,
    pub similar_to_id: Option<AppId>,
    pub similar_to_multiplier: Option<f64>,
    pub quality: f64,
    pub reviews: Reviews,
    pub in_wishlist: bool,
    pub relevance: f64,
    pub bet: bool,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub games: Vec<RecommendedGame>,
    pub family: Option<String>,
    pub saved_signals_from: Timestamp,
    pub freshness: Freshness,
    pub stale_prices: bool,
    pub empty_because: Option<Blame>,
    pub without_vote_model: bool,
    // To pass as it is to the next page
    pub continuation: Continuation,
}

pub fn find_family<'f>(families: &'f Families, name: &str) -> Result<&'f Family, SearchError> {
    let lowercase_name = name.to_lowercase();

    match families
        .families
        .iter()
        .find(|family| family.name == name)
        .or_else(|| {
            families
                .families
                .iter()
                .find(|family| family.name.to_lowercase() == lowercase_name)
        }) {
        Some(family) => Ok(family),
        None => Err(SearchError::FamilyNotFound {
            name: name.into(),
            families: families
                .families
                .iter()
                .map(|family| family.name.clone())
                .collect(),
        }),
    }
}

pub fn search_tastes(
    inputs: &SearchInputs,
    family: Option<&str>,
) -> Result<Vec<SearchTaste>, SearchError> {
    let taste_of = |family: &Family| SearchTaste {
        family: Some(family.name.clone()),
        kind: ListKind::of_family(family),
        taste: family_taste(
            family,
            inputs.catalog,
            inputs.off_catalog,
            &inputs.corrections.tag_multipliers,
        ),
    };

    match family {
        None => {
            let hours = hours_by_source(
                inputs.signals,
                inputs.votes,
                &inputs.corrections.played_elsewhere,
                &inputs.config.signals,
            );
            let taste = signals_taste(
                &hours,
                inputs.catalog,
                inputs.off_catalog,
                inputs.corrections,
                &inputs.config.signals,
            );

            if !taste.has_profile() {
                return Err(SearchError::EmptySignalsProfile);
            }

            let mut search_tastes = vec![SearchTaste {
                family: None,
                kind: ListKind::Signals,
                taste,
            }];

            // A family without a profile would leave an empty turn
            search_tastes.extend(
                inputs
                    .families
                    .families
                    .iter()
                    .map(taste_of)
                    .filter(|search_taste| search_taste.taste.has_profile()),
            );

            Ok(search_tastes)
        }
        Some(family_name) => Ok(vec![taste_of(find_family(inputs.families, family_name)?)]),
    }
}

pub fn needs_opinion(config: &Config, options: &SearchOptions) -> bool {
    options.only_bets || config.results.bet_every > 0 || config.relevance.votes_weight > 0.0
}

pub fn sort_picks(picks: &mut [Pick], sort: SortBy, catalog: &Catalog, now: Timestamp) {
    let price = |pick: &Pick| catalog.game(pick.game.id).map(|game| game.price.as_of(now));

    match sort {
        SortBy::Relevance => {}
        SortBy::Discount => picks.sort_by_key(|pick| {
            Reverse(
                price(pick)
                    .and_then(|price| price.discount)
                    .map_or(0, |discount| discount.percent),
            )
        }),
        SortBy::Price => {
            picks.sort_by_key(|pick| price(pick).map_or(u32::MAX, |price| price.final_cents()))
        }
    }
}

pub fn recommended_game(
    pick: &Pick,
    position: usize,
    tastes: &[SearchTaste],
    inputs: &SearchInputs,
    now: Timestamp,
) -> RecommendedGame {
    let id = pick.game.id;
    let game = inputs
        .catalog
        .game(id)
        .expect("ranked games are in the catalog");
    let taste = &tastes[pick.from];
    let similar = similar_to(&pick.game.references);

    RecommendedGame {
        position,
        id,
        name: game.name.clone(),
        price: game.price.as_of(now),
        store_url: format!("https://store.steampowered.com/app/{}/", id.0),
        from_family: taste.family.clone(),
        affinity: pick.game.affinity,
        top_tags: top_tags(game, &taste.taste.profile(), inputs.catalog),
        similar_to: similar
            .map(|reference| game_name(reference.id, inputs.catalog, inputs.off_catalog)),
        similar_to_id: similar.map(|reference| reference.id),
        // Only signals use game multipliers
        similar_to_multiplier: similar
            .filter(|_| taste.family.is_none())
            .and_then(|reference| inputs.corrections.game_multipliers.get(&reference.id))
            .copied(),
        quality: quality(&game.reviews),
        reviews: game.reviews,
        in_wishlist: inputs.signals.wishlist.contains(&id),
        relevance: pick.game.relevance,
        bet: pick.bet,
    }
}

// Where the last page stopped: the next page goes on from here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Continuation {
    pub shown: HashSet<AppId>,
    // The place in turn_order of the next turn of the list, or of the bets
    // with only bets
    pub turn: usize,
    pub bet_turn: usize,
    // The places already given, to keep a bet every bet_every places
    pub places: usize,
}

fn merged_lists<'a>(
    voted: &'a [Vec<RankedGame>],
    bet_lists: &'a [Vec<RankedGame>],
    shares: &[u32],
    only_bets: bool,
    bet_every: usize,
    continuation: &mut Continuation,
    limit: usize,
) -> Vec<Pick<'a>> {
    let mut seen = continuation.shown.clone();
    let mut picks = Vec::new();

    if only_bets || bet_every == 0 {
        let (lists, bet) = if only_bets {
            (bet_lists, true)
        } else {
            (voted, false)
        };
        let mut turns = Turns::new(lists, shares, continuation.turn);
        while picks.len() < limit {
            let Some(pick) = turns.next(&seen) else {
                break;
            };
            seen.insert(pick.game.id);
            picks.push(Pick { bet, ..pick });
        }
        continuation.turn = turns.turn();
    } else {
        let mut list = Turns::new(voted, shares, continuation.turn);
        let mut bets = Turns::new(bet_lists, shares, continuation.bet_turn);
        picks = with_bets(
            &mut list,
            &mut bets,
            bet_every,
            continuation.places,
            &mut seen,
            limit,
        );
        continuation.turn = list.turn();
        continuation.bet_turn = bets.turn();
    }

    continuation.places += picks.len();
    picks
}

pub fn recommend(
    inputs: &SearchInputs,
    options: &SearchOptions,
    now: Timestamp,
) -> Result<Recommendation, SearchError> {
    recommend_from(inputs, options, Continuation::default(), now)
}

pub fn recommend_from(
    inputs: &SearchInputs,
    options: &SearchOptions,
    continuation: Continuation,
    now: Timestamp,
) -> Result<Recommendation, SearchError> {
    let filters = active_filters(&options.filters, inputs.config, inputs.catalog)?;
    let mut tastes = search_tastes(inputs, options.family.as_deref())?;
    if options.signals_only && options.family.is_none() {
        tastes.truncate(1);
    }
    let filter_context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
    );
    let filter_outcome = apply_filters(inputs.catalog, &filters, &filter_context, now);

    let rankings: Vec<Vec<RankedGame>> = tastes
        .iter()
        .map(|search_taste| {
            let mut ranked = ranking(
                inputs.game_vectors,
                &search_taste.taste,
                &inputs.config.relevance,
            )
            .unwrap_or_default();
            ranked.retain(|game| filter_outcome.candidates.contains(&game.id));

            ranked
        })
        .collect();

    let opinion_needed = needs_opinion(inputs.config, options);
    let model = if opinion_needed {
        opinion_model(inputs.votes, inputs.catalog, &inputs.liked_creators())
    } else {
        None
    };
    let opinion_by_id: Option<HashMap<AppId, f64>> = model.map(|model| {
        inputs
            .catalog
            .games()
            .iter()
            .map(|game| game.id)
            .zip(opinions(&model, inputs.catalog))
            .collect()
    });

    let votes_weight = inputs.config.relevance.votes_weight;
    let (voted, bet_lists): (Vec<Vec<RankedGame>>, Vec<Vec<RankedGame>>) = match &opinion_by_id {
        Some(opinion_by_id) => rankings
            .iter()
            .map(|ranked| {
                (
                    with_votes(ranked, opinion_by_id, votes_weight),
                    bets(ranked, opinion_by_id, inputs.votes, votes_weight, now),
                )
            })
            .unzip(),
        None => (rankings, Vec::new()),
    };

    let bet_every = if opinion_by_id.is_some() {
        inputs.config.results.bet_every
    } else {
        0
    };
    let kinds: Vec<ListKind> = tastes
        .iter()
        .map(|search_taste| search_taste.kind)
        .collect();
    let limit = options.limit.unwrap_or(inputs.config.results.count);
    let mut next = continuation.clone();
    let picks = if options.sort == SortBy::Relevance {
        merged_lists(
            &voted,
            &bet_lists,
            &turn_shares(&kinds),
            options.only_bets,
            bet_every,
            &mut next,
            limit,
        )
    } else {
        // The turns don't decide the order. The whole list is sorted from the
        // start every time, so pages are pieces of one list, ties and labels included
        let mut from_start = Continuation::default();
        let mut picks = merged_lists(
            &voted,
            &bet_lists,
            &turn_shares(&kinds),
            options.only_bets,
            bet_every,
            &mut from_start,
            usize::MAX,
        );
        sort_picks(&mut picks, options.sort, inputs.catalog, now);
        picks.retain(|pick| !continuation.shown.contains(&pick.game.id));
        picks.truncate(limit);
        picks
    };
    next.shown.extend(picks.iter().map(|pick| pick.game.id));

    let games: Vec<RecommendedGame> = picks
        .iter()
        .enumerate()
        .map(|(i, pick)| {
            recommended_game(pick, continuation.shown.len() + i + 1, &tastes, inputs, now)
        })
        .collect();

    Ok(Recommendation {
        continuation: next,
        empty_because: if games.is_empty() && continuation.shown.is_empty() {
            filter_outcome.blame
        } else {
            None
        },
        games,
        family: options.family.clone(),
        saved_signals_from: inputs.signals.read_at,
        freshness: inputs.catalog.freshness,
        stale_prices: inputs.catalog.freshness.prices_are_stale(
            now,
            inputs.config.prices_max_age_hours,
            inputs.config.region,
        ),
        without_vote_model: opinion_needed && opinion_by_id.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        currency::Currency,
        families::Family,
        game::{Game, GameTag},
        game_vector::game_vectors,
        ids::TagId,
        region::Region,
        signals::LibraryGame,
        votes::{LATER_DAYS, Vote, VoteKind},
    };

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    fn game(id: u32, tag: u32) -> Game {
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
            tags: vec![GameTag {
                id: TagId(tag),
                weight: 1.0,
            }],
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn recommendation_with(
        region: Region,
        options: &SearchOptions,
        automatic: Option<&str>,
    ) -> Recommendation {
        recommendation_from(region, options, automatic, Continuation::default())
    }

    fn recommendation_from(
        region: Region,
        options: &SearchOptions,
        automatic: Option<&str>,
        continuation: Continuation,
    ) -> Recommendation {
        let catalog = Catalog::new(
            vec![
                game(1, 1),
                game(2, 1),
                game(3, 2),
                game(4, 2),
                game(5, 1),
                game(6, 2),
            ],
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 1.0), (TagId(2), 1.0)]),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let vectors = game_vectors(&catalog);
        let signals = Signals {
            read_at: NOW,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 20.0,
                last_played: None,
            }],
            borrowed: vec![],
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };
        let config = Config {
            region,
            ..Config::default()
        };
        let inputs = SearchInputs {
            catalog: &catalog,
            game_vectors: &vectors,
            signals: &signals,
            votes: &Votes::new(),
            corrections: &Corrections::default(),
            families: &Families {
                families: vec![Family {
                    name: "Puzzle".to_string(),
                    games: vec![AppId(3)],
                    automatic: automatic.map(str::to_string),
                }],
                held_out: Vec::new(),
            },
            off_catalog: &OffCatalogGames::new(),
            config: &config,
        };

        recommend_from(&inputs, options, continuation, NOW).unwrap()
    }

    fn recommendation_pages(options: &SearchOptions, page: usize) -> Vec<RecommendedGame> {
        let options = SearchOptions {
            limit: Some(page),
            ..options.clone()
        };
        let mut continuation = Continuation::default();
        let mut games = Vec::new();

        loop {
            let recommendation = recommendation_from(Region::ITALY, &options, None, continuation);
            if recommendation.games.is_empty() {
                return games;
            }
            continuation = recommendation.continuation;
            games.extend(recommendation.games);
        }
    }

    fn summary(games: &[RecommendedGame]) -> Vec<(usize, u32, Option<String>)> {
        games
            .iter()
            .map(|game| (game.position, game.id.0, game.from_family.clone()))
            .collect()
    }

    fn recommendation(region: Region, options: &SearchOptions) -> Recommendation {
        recommendation_with(region, options, None)
    }

    fn stale_prices(region: Region) -> bool {
        recommendation(region, &SearchOptions::default()).stale_prices
    }

    fn families_in(recommendation: Recommendation) -> Vec<Option<String>> {
        recommendation
            .games
            .into_iter()
            .map(|game| game.from_family)
            .collect()
    }

    fn families_of(options: &SearchOptions) -> Vec<Option<String>> {
        families_in(recommendation(Region::ITALY, options))
    }

    fn ranked(id: u32) -> RankedGame {
        RankedGame {
            id: AppId(id),
            relevance: 0.0,
            references: Vec::new(),
            affinity: 0.0,
        }
    }

    #[test]
    fn all_tastes_mix_the_signals_and_the_families() {
        let puzzle = || Some("Puzzle".to_string());

        assert_eq!(
            families_of(&SearchOptions::default()),
            [None, puzzle(), None, puzzle()]
        );
    }

    #[test]
    fn an_automatic_family_gets_a_smaller_share() {
        let recommendation =
            recommendation_with(Region::ITALY, &SearchOptions::default(), Some("2026-11-10"));

        assert_eq!(
            families_in(recommendation),
            [None, None, Some("Puzzle".to_string()), None]
        );
    }

    #[test]
    fn the_bets_take_turns_with_the_same_shares_as_the_list() {
        let shares = turn_shares(&[ListKind::Signals, ListKind::AutomaticFamily]);
        let voted = [
            (1..=10).map(ranked).collect(),
            (11..=20).map(ranked).collect(),
        ];
        let bet_lists = [
            (21..=30).map(ranked).collect(),
            (31..=40).map(ranked).collect(),
        ];
        let bets_from = |picks: Vec<Pick>| -> Vec<usize> {
            picks
                .iter()
                .filter(|pick| pick.bet)
                .map(|pick| pick.from)
                .take(5)
                .collect()
        };

        let merged = |only_bets, bet_every| {
            let mut continuation = Continuation::default();
            merged_lists(
                &voted,
                &bet_lists,
                &shares,
                only_bets,
                bet_every,
                &mut continuation,
                usize::MAX,
            )
        };

        let only_bets = merged(true, 0);
        let mixed = merged(false, 2);
        let list: Vec<usize> = merged(false, 0)
            .iter()
            .map(|pick| pick.from)
            .take(5)
            .collect();

        assert_eq!(list, [0, 0, 1, 0, 0]);
        assert_eq!(bets_from(only_bets), [0, 0, 1, 0, 0]);
        assert_eq!(bets_from(mixed), [0, 0, 1, 0, 0]);
    }

    type Placed = (u32, usize, bool);

    fn placed(picks: &[Pick]) -> Vec<Placed> {
        picks
            .iter()
            .map(|pick| (pick.game.id.0, pick.from, pick.bet))
            .collect()
    }

    // The pages one after the other, without new votes in between
    fn in_pages(
        voted: &[Vec<RankedGame>],
        bet_lists: &[Vec<RankedGame>],
        kinds: &[ListKind],
        (only_bets, bet_every): (bool, usize),
        page: usize,
        places: usize,
    ) -> Vec<Placed> {
        let shares = turn_shares(kinds);
        let mut continuation = Continuation::default();
        let mut all = Vec::new();

        while all.len() < places {
            let limit = page.min(places - all.len());
            let picks = merged_lists(
                voted,
                bet_lists,
                &shares,
                only_bets,
                bet_every,
                &mut continuation,
                limit,
            );
            if picks.is_empty() {
                break;
            }
            continuation
                .shown
                .extend(picks.iter().map(|pick| pick.game.id));
            all.extend(placed(&picks));
        }

        all
    }

    fn ranked_list(ids: impl IntoIterator<Item = u32>) -> Vec<RankedGame> {
        ids.into_iter().map(ranked).collect()
    }

    // The signals, a family of the user and an automatic one that finishes
    // early. The bets are in the lists too, some of them near the top, and
    // games are shared between lists.
    fn overlapping_lists() -> (Vec<Vec<RankedGame>>, Vec<Vec<RankedGame>>) {
        let voted = vec![
            ranked_list(1..=60),
            ranked_list([5, 61, 7, 62, 63, 2, 64, 65, 66, 67, 68, 69, 70, 71, 72]),
            ranked_list([100, 3, 101, 102, 103]),
        ];
        let bet_lists = vec![
            ranked_list([7, 3, 12, 40, 55]),
            ranked_list([62, 5, 70]),
            ranked_list([101]),
        ];
        (voted, bet_lists)
    }

    #[test]
    fn pages_that_go_on_give_the_same_places_as_one_page() {
        let (voted, bet_lists) = overlapping_lists();
        let kinds = [
            ListKind::Signals,
            ListKind::UserFamily,
            ListKind::AutomaticFamily,
        ];

        for bets in [(false, 10), (false, 3), (false, 0), (true, 0)] {
            let one_page = in_pages(&voted, &bet_lists, &kinds, bets, usize::MAX, 20);
            assert_eq!(one_page.len(), if bets.0 { 9 } else { 20 });

            for page in [10, 7, 5, 1] {
                assert_eq!(
                    in_pages(&voted, &bet_lists, &kinds, bets, page, 20),
                    one_page,
                    "pages of {page}, bets {bets:?}"
                );
            }
        }
    }

    #[test]
    fn a_list_finished_halfway_through_a_page_moves_the_turns_as_in_one_page() {
        let (voted, bet_lists) = overlapping_lists();
        let kinds = [
            ListKind::Signals,
            ListKind::UserFamily,
            ListKind::AutomaticFamily,
        ];

        // The automatic family runs out of games well before the end
        let one_page = in_pages(&voted, &bet_lists, &kinds, (false, 10), usize::MAX, 80);
        let last_automatic = one_page.iter().rposition(|&(_, from, _)| from == 2);
        assert!(last_automatic.is_some_and(|at| at < 40));

        for page in [10, 3] {
            assert_eq!(
                in_pages(&voted, &bet_lists, &kinds, (false, 10), page, 80),
                one_page
            );
        }
    }

    #[test]
    fn short_pages_give_every_automatic_family_its_places() {
        let kinds = [
            ListKind::Signals,
            ListKind::AutomaticFamily,
            ListKind::AutomaticFamily,
            ListKind::AutomaticFamily,
            ListKind::AutomaticFamily,
        ];
        let voted: Vec<Vec<RankedGame>> = (0..5)
            .map(|i| ranked_list(i * 1000..i * 1000 + 200))
            .collect();

        let places = in_pages(&voted, &[], &kinds, (false, 0), 5, 100);

        for family in 1..=4 {
            let first_twenty = |round: usize| {
                places[round * 20..(round + 1) * 20]
                    .iter()
                    .filter(|&&(_, from, _)| from == family)
                    .count()
            };
            assert!((0..5).all(|round| first_twenty(round) == 1));
        }
    }

    #[test]
    fn short_pages_bring_a_bet_every_ten_places() {
        let voted = [ranked_list(1..=200)];
        let bet_lists = [ranked_list(301..=400)];

        let places = in_pages(
            &voted,
            &bet_lists,
            &[ListKind::Signals],
            (false, 10),
            5,
            100,
        );
        let bets_at: Vec<usize> = places
            .iter()
            .enumerate()
            .filter(|(_, (_, _, bet))| *bet)
            .map(|(i, _)| i + 1)
            .collect();

        assert_eq!(bets_at, (1..=10).map(|n| n * 10).collect::<Vec<_>>());
    }

    // Enough votes for an opinion model: the games with tag 3 are liked and
    // those with tag 4 are not, so the candidates with tag 4 become bets.
    // Tag 1 is the taste of the signals, tag 2 of a family of the user and
    // tag 5 of an automatic family.
    fn recommendation_with_votes(
        sort: SortBy,
        limit: usize,
        continuation: Continuation,
    ) -> Recommendation {
        let tagged = |id: u32, tags: &[u32]| Game {
            tags: tags
                .iter()
                .map(|&tag| GameTag {
                    id: TagId(tag),
                    weight: 1.0,
                })
                .collect(),
            ..game(id, 0)
        };
        let mut games = vec![tagged(1, &[1]), tagged(2, &[2]), tagged(3, &[5])];
        games.extend((10..20).map(|id| tagged(id, &[1, 3])));
        games.extend((20..30).map(|id| tagged(id, &[1, 4])));
        games.extend((100..160).map(|id| {
            let taste = [1, 2, 5][(id % 3) as usize];
            let opinion = if id % 4 == 0 { 4 } else { 3 };
            tagged(id, &[taste, opinion])
        }));
        let catalog = Catalog::new(
            games,
            BTreeMap::new(),
            (1..=5).map(|tag| (TagId(tag), 1.0)).collect(),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let vectors = game_vectors(&catalog);
        let signals = Signals {
            read_at: NOW,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 20.0,
                last_played: None,
            }],
            borrowed: vec![],
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };
        let vote = |kind| Vote {
            kind,
            date: "2026-11-10".to_string(),
            at: None,
            bet: false,
            genre_ok: false,
        };
        let votes: Votes = (10..20)
            .map(|id| (AppId(id), vote(VoteKind::WouldTry)))
            .chain((20..30).map(|id| (AppId(id), vote(VoteKind::NotForMe))))
            .collect();
        let family = |name: &str, game: u32, automatic: Option<&str>| Family {
            name: name.to_string(),
            games: vec![AppId(game)],
            automatic: automatic.map(str::to_string),
        };
        let config = Config::default();
        let inputs = SearchInputs {
            catalog: &catalog,
            game_vectors: &vectors,
            signals: &signals,
            votes: &votes,
            corrections: &Corrections::default(),
            families: &Families {
                families: vec![
                    family("Puzzle", 2, None),
                    family("Racing", 3, Some("2026-11-10")),
                ],
                held_out: Vec::new(),
            },
            off_catalog: &OffCatalogGames::new(),
            config: &config,
        };
        let options = SearchOptions {
            sort,
            limit: Some(limit),
            ..SearchOptions::default()
        };

        recommend_from(&inputs, &options, continuation, NOW).unwrap()
    }

    fn shown(games: &[RecommendedGame]) -> Vec<(usize, u32, Option<String>, bool)> {
        games
            .iter()
            .map(|game| (game.position, game.id.0, game.from_family.clone(), game.bet))
            .collect()
    }

    fn pages_with_votes(sort: SortBy, page: usize, places: usize) -> Vec<RecommendedGame> {
        let mut continuation = Continuation::default();
        let mut games = Vec::new();
        while games.len() < places {
            let recommendation =
                recommendation_with_votes(sort, page.min(places - games.len()), continuation);
            continuation = recommendation.continuation;
            games.extend(recommendation.games);
        }
        games
    }

    fn shown_without_voted(votes: &Votes) -> Vec<u32> {
        let catalog = Catalog::new(
            (1..=6).map(|id| game(id, 1)).collect(),
            BTreeMap::new(),
            BTreeMap::from([(TagId(1), 1.0)]),
            Freshness {
                prices_updated_at: NOW,
                tags_updated_at: NOW,
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let vectors = game_vectors(&catalog);
        let signals = Signals {
            read_at: NOW,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 20.0,
                last_played: None,
            }],
            borrowed: vec![],
            recent: Vec::new(),
            wishlist: Vec::new(),
            followed: Vec::new(),
        };
        let config = Config::default();
        let inputs = SearchInputs {
            catalog: &catalog,
            game_vectors: &vectors,
            signals: &signals,
            votes,
            corrections: &Corrections::default(),
            families: &Families::default(),
            off_catalog: &OffCatalogGames::new(),
            config: &config,
        };
        let options = SearchOptions {
            filters: FilterOptions {
                without_voted: true,
                ..FilterOptions::default()
            },
            ..SearchOptions::default()
        };

        let mut ids: Vec<u32> = recommend(&inputs, &options, NOW)
            .unwrap()
            .games
            .iter()
            .map(|game| game.id.0)
            .collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn a_later_comes_back_once_its_days_are_over() {
        let day = 24 * 60 * 60 * 1000;
        let later = |at: i64| Vote {
            kind: VoteKind::Later,
            date: "2026-10-01".to_string(),
            at: Some(Timestamp(at)),
            bet: false,
            genre_ok: false,
        };
        let votes = Votes::from([
            (AppId(2), later(NOW.0 - day)),
            (AppId(3), later(NOW.0 - LATER_DAYS * day)),
            (
                AppId(4),
                Vote {
                    kind: VoteKind::NotForMe,
                    ..later(NOW.0 - 2 * LATER_DAYS * day)
                },
            ),
        ]);

        assert_eq!(shown_without_voted(&votes), [3, 5, 6]);
    }

    #[test]
    fn pages_with_votes_and_bets_go_on_as_one_page() {
        let one_page = recommendation_with_votes(SortBy::Relevance, 40, Continuation::default());

        // The fixture reaches what it is for
        assert!(!one_page.without_vote_model);
        assert_eq!(one_page.games.len(), 40);
        assert_eq!(
            one_page.games.iter().filter(|game| game.bet).count(),
            4,
            "a bet every 10 places"
        );
        for family in ["Puzzle", "Racing"] {
            assert!(
                one_page
                    .games
                    .iter()
                    .any(|game| game.from_family.as_deref() == Some(family))
            );
        }

        for page in [10, 7, 3] {
            let games = pages_with_votes(SortBy::Relevance, page, 40);

            assert_eq!(shown(&games), shown(&one_page.games), "pages of {page}");
        }
    }

    #[test]
    fn pages_sorted_by_price_are_pieces_of_one_page() {
        // All the games of the fixture have the same price: only the ties decide
        let one_page = recommendation_with_votes(SortBy::Price, 40, Continuation::default());
        assert_eq!(one_page.games.len(), 40);

        for page in [10, 7, 3] {
            let games = pages_with_votes(SortBy::Price, page, 40);

            assert_eq!(shown(&games), shown(&one_page.games), "pages of {page}");
        }
    }

    #[test]
    fn a_recommendation_goes_on_from_its_continuation() {
        let one_page = recommendation(Region::ITALY, &SearchOptions::default());

        let pages = recommendation_pages(&SearchOptions::default(), 1);

        assert_eq!(summary(&pages), summary(&one_page.games));
        assert_eq!(one_page.continuation.places, one_page.games.len());
    }

    #[test]
    fn a_new_page_sorted_by_price_does_not_repeat_the_games_shown() {
        let options = SearchOptions {
            sort: SortBy::Price,
            ..SearchOptions::default()
        };
        let one_page = recommendation(Region::ITALY, &options);

        let pages = recommendation_pages(&options, 1);

        assert_eq!(summary(&pages), summary(&one_page.games));
    }

    #[test]
    fn signals_only_leaves_the_families_out() {
        let options = SearchOptions {
            signals_only: true,
            ..SearchOptions::default()
        };

        let families = families_of(&options);

        assert!(!families.is_empty());
        assert!(families.iter().all(Option::is_none));
    }

    #[test]
    fn prices_of_another_region_than_the_config_are_stale() {
        assert!(stale_prices(Region::new("US").unwrap()));
        assert!(!stale_prices(Region::ITALY));
    }
}
