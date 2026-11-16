use gamepalate_core::{
    effective_hours::{HoursSource, hours_by_source},
    filters::{DiscoverMode, Filter, FilterContext, active_filters, first_failed, hits_ban},
    game::Game,
    ids::{AppId, TagId},
    search::{SearchInputs, SearchOptions, recommend},
    timestamp::Timestamp,
};

use crate::names::tag_name;

/// Where a game stands in Discover, as if you opened it now from the start. A game of your
/// backlog is looked for in the Backlog, any other in New.
#[derive(Debug, Clone, PartialEq)]
pub enum DiscoverPlace {
    Shown {
        position: usize,
    },
    LeftOut(LeftOut),
    /// It passes the filters, but no list of yours has it.
    NoMatch,
}

/// The first filter that keeps a game out of Discover, with what its text needs.
#[derive(Debug, Clone, PartialEq)]
pub enum LeftOut {
    Owned,
    /// You played it, for this many hours: it's not a game for Discover any more.
    Played(f64),
    Wishlist,
    Followed,
    FamilyGame,
    Hidden,
    Voted,
    BannedTag(String),
    FewReviews(u32),
    ManyReviews(u32),
    LowPositive(u8),
    EarlyAccess,
    Price,
    Discount,
    MissingTag(String),
    ExcludedTag(String),
}

pub(crate) fn discover_place(
    id: AppId,
    options: &SearchOptions,
    inputs: &SearchInputs,
    now: Timestamp,
) -> Option<DiscoverPlace> {
    let game = inputs.catalog.game(id)?;
    let context = FilterContext::new(
        inputs.signals,
        inputs.families,
        inputs.votes,
        inputs.corrections,
        &inputs.config.signals,
    );
    let mode = if context.backlog.contains(&id) {
        DiscoverMode::Backlog
    } else {
        DiscoverMode::New
    };
    let filters = active_filters(&options.filters, inputs.config, inputs.catalog, mode).ok()?;
    if let Some(filter) = first_failed(game, &filters, &context, now) {
        return Some(DiscoverPlace::LeftOut(left_out(filter, game, inputs)));
    }

    let every_game = SearchOptions {
        limit: Some(usize::MAX),
        mode,
        ..options.clone()
    };
    let recommendation = recommend(inputs, &every_game, now).ok()?;
    // Every game is in the lists, the ones without affinity at the bottom
    Some(
        match recommendation
            .games
            .iter()
            .find(|recommended| recommended.id == id)
        {
            Some(recommended) if recommended.affinity > 0.0 => DiscoverPlace::Shown {
                position: recommended.position,
            },
            _ => DiscoverPlace::NoMatch,
        },
    )
}

fn left_out(filter: &Filter, game: &Game, inputs: &SearchInputs) -> LeftOut {
    let has_tag = |tag_id: TagId| game.tags.iter().any(|tag| tag.id == tag_id);
    let name =
        |tag_id: Option<TagId>| tag_id.map_or_else(String::new, |id| tag_name(id, inputs.catalog));

    match filter {
        Filter::Owned => played_hours(game.id, inputs).map_or(LeftOut::Owned, LeftOut::Played),
        // The game is not in the backlog, so it can't be the filter that fails there
        Filter::Backlog => LeftOut::Owned,
        Filter::WishedOrFollowed => {
            if inputs.signals.wishlist.contains(&game.id) {
                LeftOut::Wishlist
            } else {
                LeftOut::Followed
            }
        }
        Filter::FamilyGames => LeftOut::FamilyGame,
        Filter::Hidden => LeftOut::Hidden,
        Filter::NotForMe | Filter::Voted => LeftOut::Voted,
        Filter::BannedTags => LeftOut::BannedTag(name(
            inputs
                .corrections
                .banned_tags
                .iter()
                .find(|&(&tag_id, &ban)| hits_ban(game, tag_id, ban))
                .map(|(&tag_id, _)| tag_id),
        )),
        Filter::MinReviews(min) => LeftOut::FewReviews(*min),
        Filter::MaxReviews(max) => LeftOut::ManyReviews(*max),
        Filter::MinPositivePercent(min) => LeftOut::LowPositive(*min),
        Filter::EarlyAccess => LeftOut::EarlyAccess,
        Filter::MinPrice(_) | Filter::MaxPrice(_) => LeftOut::Price,
        Filter::OnlyDiscounted | Filter::MinDiscount(_) => LeftOut::Discount,
        Filter::RequiredTags(tag_ids) => LeftOut::MissingTag(name(
            tag_ids.iter().copied().find(|&tag_id| !has_tag(tag_id)),
        )),
        Filter::ExcludedTags(tag_ids) => LeftOut::ExcludedTag(name(
            tag_ids.iter().copied().find(|&tag_id| has_tag(tag_id)),
        )),
    }
}

/// The hours of Steam for a game that is past the backlog. A Would try, a wishlist, a followed
/// game or one played elsewhere has no hours of yours behind it, so it has none.
fn played_hours(id: AppId, inputs: &SearchInputs) -> Option<f64> {
    let hours = hours_by_source(
        inputs.signals,
        inputs.votes,
        &inputs.corrections.played_elsewhere,
        &inputs.config.signals,
    );
    let &(effective, source) = hours.get(&id)?;
    if source != HoursSource::Played || effective < inputs.config.signals.min_hours {
        return None;
    }
    let signals = inputs.signals;
    signals
        .recent
        .iter()
        .find(|game| game.id == id)
        .map(|game| game.total_hours)
        .or_else(|| {
            signals
                .library
                .iter()
                .chain(&signals.borrowed)
                .find(|game| game.id == id)
                .map(|game| game.total_hours)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gamepalate_core::{
        catalog::{Catalog, Freshness, TagName},
        config::Config,
        corrections::{Ban, Corrections},
        currency::Currency,
        families::Families,
        filters::{DiscoverMode, FilterOptions},
        game::{GameTag, Price, Reviews},
        game_vector::{GameVector, game_vectors},
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, Signals},
        votes::{Vote, VoteKind, Votes},
    };

    use super::*;

    // 3 November 2026
    const NOW: Timestamp = Timestamp(1_793_664_000_000);

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
                .enumerate()
                .map(|(i, &tag)| GameTag {
                    id: TagId(tag),
                    weight: 1.0 - i as f64 * 0.1,
                })
                .collect(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    struct Fixture {
        catalog: Catalog,
        vectors: Vec<GameVector>,
        signals: Signals,
        votes: Votes,
        corrections: Corrections,
        families: Families,
        off_catalog: OffCatalogGames,
        config: Config,
    }

    impl Fixture {
        fn new() -> Fixture {
            let names = ["Roguelike", "Horror", "Puzzle"];
            let catalog = Catalog::new(
                vec![
                    game(1, &[1]),
                    game(2, &[1, 2]),
                    game(3, &[1, 3]),
                    game(4, &[3]),
                    game(5, &[1]),
                    Game {
                        reviews: Reviews {
                            total: 40,
                            positive: 39,
                        },
                        ..game(6, &[1, 2])
                    },
                ],
                names
                    .iter()
                    .enumerate()
                    .map(|(i, &name)| {
                        (
                            TagId(i as u32 + 1),
                            TagName {
                                english: name.to_string(),
                                italian: name.to_string(),
                            },
                        )
                    })
                    .collect(),
                (1..=3).map(|id| (TagId(id), 1.0)).collect(),
                Freshness {
                    prices_updated_at: NOW,
                    tags_updated_at: NOW,
                    region: Region::ITALY,
                },
                Currency::italian(),
            );
            Fixture {
                vectors: game_vectors(&catalog),
                catalog,
                signals: Signals {
                    read_at: NOW,
                    library: vec![LibraryGame {
                        id: AppId(1),
                        total_hours: 20.0,
                        last_played: None,
                    }],
                    borrowed: vec![],
                    recent: vec![],
                    wishlist: vec![],
                    followed: vec![],
                },
                votes: Votes::new(),
                corrections: Corrections::default(),
                families: Families::default(),
                off_catalog: OffCatalogGames::new(),
                config: Config::default(),
            }
        }

        fn inputs(&self) -> SearchInputs<'_> {
            SearchInputs {
                catalog: &self.catalog,
                game_vectors: &self.vectors,
                signals: &self.signals,
                votes: &self.votes,
                corrections: &self.corrections,
                families: &self.families,
                off_catalog: &self.off_catalog,
                config: &self.config,
            }
        }

        fn place(&self, id: u32, options: &SearchOptions) -> Option<DiscoverPlace> {
            discover_place(AppId(id), options, &self.inputs(), NOW)
        }
    }

    fn without_voted() -> SearchOptions {
        SearchOptions {
            filters: FilterOptions {
                without_voted: true,
                ..FilterOptions::default()
            },
            ..SearchOptions::default()
        }
    }

    #[test]
    fn the_first_filter_that_leaves_a_game_out_is_the_one_said() {
        let mut fixture = Fixture::new();
        fixture.corrections.hidden_games = BTreeSet::from([AppId(6)]);

        // Hidden and with too few reviews: Hidden comes first
        assert_eq!(
            fixture.place(6, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Hidden))
        );

        fixture.corrections.hidden_games.clear();
        assert_eq!(
            fixture.place(6, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::FewReviews(1000)))
        );
        assert_eq!(
            fixture.place(1, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Played(20.0)))
        );
    }

    fn unplayed(id: u32) -> LibraryGame {
        LibraryGame {
            id: AppId(id),
            total_hours: 0.5,
            last_played: None,
        }
    }

    #[test]
    fn a_game_of_the_backlog_has_its_place_in_the_backlog() {
        let mut fixture = Fixture::new();
        // 6 has 40 reviews, too few for New, and a game of the backlog doesn't need them
        fixture.signals.library.extend([unplayed(2), unplayed(6)]);
        let options = without_voted();
        let in_backlog = recommend(
            &fixture.inputs(),
            &SearchOptions {
                limit: Some(100),
                mode: DiscoverMode::Backlog,
                ..options.clone()
            },
            NOW,
        )
        .unwrap();

        assert_eq!(in_backlog.games.len(), 2);
        for recommended in &in_backlog.games {
            assert_eq!(
                fixture.place(recommended.id.0, &options),
                Some(DiscoverPlace::Shown {
                    position: recommended.position
                })
            );
        }
    }

    #[test]
    fn a_game_of_the_backlog_is_left_out_by_the_filters_of_the_backlog() {
        let mut fixture = Fixture::new();
        fixture.signals.library.push(unplayed(2));
        fixture.corrections.hidden_games = BTreeSet::from([AppId(2)]);

        assert_eq!(
            fixture.place(2, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Hidden))
        );
    }

    #[test]
    fn a_game_played_a_little_is_out_of_the_backlog_with_its_hours() {
        let mut fixture = Fixture::new();
        fixture.signals.library.push(LibraryGame {
            id: AppId(2),
            total_hours: 2.5,
            last_played: None,
        });

        assert_eq!(
            fixture.place(2, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Played(2.5)))
        );
    }

    #[test]
    fn a_game_played_elsewhere_or_with_a_would_try_has_no_hours_of_yours() {
        let mut fixture = Fixture::new();
        fixture.signals.library.extend([unplayed(2), unplayed(3)]);
        fixture.corrections.played_elsewhere = BTreeSet::from([AppId(2)]);
        fixture.votes = Votes::from([(AppId(3), Vote::new(VoteKind::WouldTry, "2026-11-01"))]);

        for id in [2, 3] {
            assert_eq!(
                fixture.place(id, &without_voted()),
                Some(DiscoverPlace::LeftOut(LeftOut::Owned))
            );
        }
    }

    #[test]
    fn a_wishlist_or_followed_game_is_not_in_new() {
        let mut fixture = Fixture::new();
        fixture.signals.wishlist = vec![AppId(2), AppId(3)];
        fixture.signals.followed = vec![AppId(3), AppId(4)];

        assert_eq!(
            fixture.place(2, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Wishlist))
        );
        assert_eq!(
            fixture.place(3, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Wishlist))
        );
        assert_eq!(
            fixture.place(4, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Followed))
        );
    }

    #[test]
    fn a_banned_tag_and_a_required_one_are_named() {
        let mut fixture = Fixture::new();
        fixture.corrections.banned_tags = BTreeMap::from([(TagId(2), Ban::Everywhere)]);

        assert_eq!(
            fixture.place(2, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::BannedTag(
                "Horror".to_string()
            )))
        );

        let mut options = without_voted();
        options.filters.tags = vec!["Puzzle".to_string()];
        assert_eq!(
            fixture.place(5, &options),
            Some(DiscoverPlace::LeftOut(LeftOut::MissingTag(
                "Puzzle".to_string()
            )))
        );

        let mut options = without_voted();
        options.filters.without_tags = vec!["Puzzle".to_string()];
        assert_eq!(
            fixture.place(3, &options),
            Some(DiscoverPlace::LeftOut(LeftOut::ExcludedTag(
                "Puzzle".to_string()
            )))
        );
    }

    #[test]
    fn the_position_is_the_one_of_discover() {
        let fixture = Fixture::new();
        let options = without_voted();
        let recommendation = recommend(
            &fixture.inputs(),
            &SearchOptions {
                limit: Some(100),
                ..options.clone()
            },
            NOW,
        )
        .unwrap();

        assert!(recommendation.games.len() >= 3);
        for recommended in recommendation
            .games
            .iter()
            .filter(|recommended| recommended.affinity > 0.0)
        {
            assert_eq!(
                fixture.place(recommended.id.0, &options),
                Some(DiscoverPlace::Shown {
                    position: recommended.position
                })
            );
        }
    }

    #[test]
    fn a_game_far_from_your_tastes_has_no_match() {
        let fixture = Fixture::new();

        assert_eq!(
            fixture.place(4, &without_voted()),
            Some(DiscoverPlace::NoMatch)
        );
    }

    #[test]
    fn a_vote_leaves_the_game_out_until_a_later_is_over() {
        let mut fixture = Fixture::new();
        fixture.votes = Votes::from([
            (AppId(2), Vote::new(VoteKind::NotForMe, "2026-11-01")),
            (AppId(3), Vote::new(VoteKind::Later, "2026-11-01")),
            (AppId(5), Vote::new(VoteKind::Later, "2026-09-01")),
        ]);

        assert_eq!(
            fixture.place(2, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Voted))
        );
        assert_eq!(
            fixture.place(3, &without_voted()),
            Some(DiscoverPlace::LeftOut(LeftOut::Voted))
        );
        assert!(matches!(
            fixture.place(5, &without_voted()),
            Some(DiscoverPlace::Shown { .. })
        ));
    }

    #[test]
    fn a_game_out_of_the_catalog_has_no_place() {
        assert_eq!(Fixture::new().place(99, &without_voted()), None);
    }
}
