use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use gamepalate_core::{
    corrections::Corrections,
    effective_hours::{HoursSource, hours_by_source},
    ids::AppId,
    profile::{heaviest_tags, profile_games},
    search::SearchInputs,
    taste::{game_name, signals_taste},
};
use gamepalate_store::{
    corrections::CorrectionsError,
    corrections_edit::{CorrectionsEditor, EntryValue, Section},
};

use crate::{
    families::automatic_family_names,
    names::{game_label, tag_name},
};

const GAME_TAGS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryView {
    pub games: Vec<LibraryGameView>,
    pub families: Vec<String>,
    pub automatic_families: Vec<String>,
    pub min_hours: f64,
    pub wishlist_hours: f64,
    pub followed_hours: f64,
    pub elsewhere_hours: f64,
    pub dampen: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryGameView {
    pub id: AppId,
    pub name: String,
    pub owned_hours: Option<f64>,
    pub borrowed: bool,
    pub recent: bool,
    pub wishlist: bool,
    pub followed: bool,
    pub elsewhere: bool,
    pub elsewhere_only: bool,
    pub hours: f64,
    pub source: HoursSource,
    pub counts: bool,
    pub share: f64,
    pub tags: Vec<String>,
    pub correction: GameCorrection,
    pub families: Vec<String>,
    pub held_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GameCorrection {
    pub multiplier: Option<f64>,
    pub hidden: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PlayedElsewhereError {
    #[error("{id} is not a valid Steam appid")]
    InvalidAppId { id: u32 },

    #[error("could not save the game played elsewhere in {}", path.display())]
    Save {
        path: std::path::PathBuf,
        #[source]
        source: Box<CorrectionsError>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum GameCorrectionError {
    #[error(
        "the factor of a game must be from 0 to 1, where 0 means it does not count, but it is {factor}"
    )]
    FactorOutOfRange { factor: f64 },

    #[error("{game} is not in your signals: dampening it would have no effect")]
    NotInSignals { game: String },

    #[error(
        "{game} has {hours:.2} effective hours, below the threshold of {min_hours} hours: dampening it would have no effect"
    )]
    BelowThreshold {
        game: String,
        hours: f64,
        min_hours: f64,
    },

    #[error("could not save the correction in {}", path.display())]
    Save {
        path: std::path::PathBuf,
        #[source]
        source: Box<CorrectionsError>,
    },
}

pub(crate) fn library_view(inputs: &SearchInputs) -> LibraryView {
    let signals = inputs.signals;
    let config = &inputs.config.signals;
    let hours = hours_by_source(
        signals,
        inputs.votes,
        &inputs.corrections.played_elsewhere,
        config,
    );
    let taste = signals_taste(
        &hours,
        inputs.catalog,
        inputs.off_catalog,
        inputs.corrections,
        config,
    );
    let weights: BTreeMap<AppId, f64> = profile_games(&taste, &hours)
        .into_iter()
        .map(|game| (game.id, game.weight))
        .collect();
    let total: f64 = weights.values().sum();

    let games = hours
        .iter()
        .filter(|&(&id, _)| in_signals(id, inputs))
        .map(|(&id, &(game_hours, source))| {
            let weight = weights.get(&id).copied().unwrap_or(0.0);
            let tags = taste
                .games
                .iter()
                .find(|game| game.id == id)
                .map(|game| {
                    heaviest_tags(&game.tag_vector)
                        .into_iter()
                        .filter(|&(_, value)| value > 0.0)
                        .take(GAME_TAGS)
                        .map(|(tag, _)| tag_name(tag, inputs.catalog))
                        .collect()
                })
                .unwrap_or_default();

            let elsewhere = inputs.corrections.played_elsewhere.contains(&id);
            LibraryGameView {
                id,
                name: game_name(id, inputs.catalog, inputs.off_catalog),
                owned_hours: signals
                    .library
                    .iter()
                    .find(|game| game.id == id)
                    .map(|game| game.total_hours),
                borrowed: signals.borrowed.iter().any(|game| game.id == id),
                recent: signals.recent.iter().any(|game| game.id == id),
                wishlist: signals.wishlist.contains(&id),
                followed: signals.followed.contains(&id),
                elsewhere,
                elsewhere_only: elsewhere && !in_steam_signals(id, inputs),
                hours: game_hours,
                source,
                counts: weight > 0.0,
                share: if total == 0.0 { 0.0 } else { weight / total },
                tags,
                correction: game_correction(id, inputs.corrections),
                families: inputs
                    .families
                    .families
                    .iter()
                    .filter(|family| family.games.contains(&id))
                    .map(|family| family.name.clone())
                    .collect(),
                held_out: inputs.families.held_out.contains(&id),
            }
        })
        .collect();

    LibraryView {
        games,
        families: inputs
            .families
            .families
            .iter()
            .map(|family| family.name.clone())
            .collect(),
        automatic_families: automatic_family_names(inputs.families),
        min_hours: config.min_hours,
        wishlist_hours: config.wishlist_hours,
        followed_hours: config.followed_hours,
        elsewhere_hours: config.elsewhere_hours,
        dampen: inputs.config.corrections.dampen,
    }
}

fn in_signals(id: AppId, inputs: &SearchInputs) -> bool {
    in_steam_signals(id, inputs) || inputs.corrections.played_elsewhere.contains(&id)
}

fn in_steam_signals(id: AppId, inputs: &SearchInputs) -> bool {
    let signals = inputs.signals;
    signals.played().any(|played| played == id)
        || signals.recent.iter().any(|game| game.id == id)
        || signals.wishlist.contains(&id)
        || signals.followed.contains(&id)
}

pub(crate) fn library_ids(inputs: &SearchInputs) -> BTreeSet<AppId> {
    let mut ids: BTreeSet<AppId> = inputs.signals.ids().into_iter().collect();
    ids.extend(&inputs.corrections.played_elsewhere);
    ids
}

pub fn game_correction(id: AppId, corrections: &Corrections) -> GameCorrection {
    GameCorrection {
        multiplier: corrections.game_multipliers.get(&id).copied(),
        hidden: corrections.hidden_games.contains(&id),
    }
}

pub fn check_game_multiplier(
    id: AppId,
    factor: f64,
    inputs: &SearchInputs,
) -> Result<(), GameCorrectionError> {
    if !(0.0..=1.0).contains(&factor) {
        return Err(GameCorrectionError::FactorOutOfRange { factor });
    }
    let game = || game_label(id, inputs.catalog, inputs.off_catalog);
    let config = &inputs.config.signals;
    let hours = hours_by_source(
        inputs.signals,
        inputs.votes,
        &inputs.corrections.played_elsewhere,
        config,
    );
    match hours.get(&id) {
        None => Err(GameCorrectionError::NotInSignals { game: game() }),
        Some(&(hours, _)) if hours < config.min_hours => Err(GameCorrectionError::BelowThreshold {
            game: game(),
            hours,
            min_hours: config.min_hours,
        }),
        Some(_) => Ok(()),
    }
}

pub(crate) fn corrected_game(
    path: &Path,
    id: AppId,
    correction: GameCorrection,
    inputs: &SearchInputs,
) -> Result<Corrections, GameCorrectionError> {
    let before = game_correction(id, inputs.corrections);
    if let Some(factor) = correction.multiplier
        && before.multiplier != Some(factor)
    {
        check_game_multiplier(id, factor, inputs)?;
    }
    if before == correction {
        return Ok(inputs.corrections.clone());
    }

    let name = game_name(id, inputs.catalog, inputs.off_catalog);
    let comment = (name != format!("#{}", id.0)).then_some(name);
    let save_error = |source| GameCorrectionError::Save {
        path: path.to_path_buf(),
        source: Box::new(source),
    };
    let mut editor = CorrectionsEditor::open(path).map_err(save_error)?;
    match correction.multiplier {
        Some(factor) => editor.set(
            Section::GameMultipliers,
            id.0,
            EntryValue::Float(factor),
            comment.as_deref(),
        ),
        None => {
            editor.remove(Section::GameMultipliers, id.0);
        }
    }
    if correction.hidden {
        editor.set(
            Section::HiddenGames,
            id.0,
            EntryValue::Bool(true),
            comment.as_deref(),
        );
    } else {
        editor.remove(Section::HiddenGames, id.0);
    }
    editor.save().map_err(save_error)
}

pub(crate) fn changed_played_elsewhere(
    path: &Path,
    id: AppId,
    played: bool,
    corrections: &Corrections,
    name: Option<&str>,
) -> Result<Corrections, PlayedElsewhereError> {
    if id.0 == 0 {
        return Err(PlayedElsewhereError::InvalidAppId { id: id.0 });
    }
    if corrections.played_elsewhere.contains(&id) == played {
        return Ok(corrections.clone());
    }

    let save_error = |source| PlayedElsewhereError::Save {
        path: path.to_path_buf(),
        source: Box::new(source),
    };
    let mut editor = CorrectionsEditor::open(path).map_err(save_error)?;
    if played {
        editor.set(Section::PlayedElsewhere, id.0, EntryValue::Bool(true), name);
    } else {
        editor.remove(Section::PlayedElsewhere, id.0);
    }
    editor.save().map_err(save_error)
}

#[cfg(test)]
mod tests {
    use gamepalate_core::{
        catalog::{Catalog, Freshness, TagName},
        config::Config,
        currency::Currency,
        families::{Families, Family},
        game::{Game, GameTag, Price, Reviews},
        ids::TagId,
        off_catalog::{OffCatalogGame, OffCatalogGames},
        region::Region,
        signals::{LibraryGame, RecentGame, Signals},
        timestamp::Timestamp,
        votes::{Vote, VoteKind, Votes},
    };

    use super::*;

    fn game(id: u32, name: &str, tags: &[u32]) -> Game {
        Game {
            id: AppId(id),
            name: name.to_string(),
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

    fn catalog() -> Catalog {
        let names = ["Exploration", "Puzzle", "Pixel Graphics", "Roguelike"];
        Catalog::new(
            vec![
                game(1, "Outer Wilds", &[1, 2, 3, 4]),
                game(2, "Aseprite", &[3]),
                game(3, "Portal 2", &[2]),
                game(4, "Hades", &[4]),
                game(5, "Celeste", &[3]),
                game(6, "Balatro", &[4]),
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
            (1..=4).map(|id| (TagId(id), 1.0)).collect(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn signals() -> Signals {
        let owned = |id, total_hours| LibraryGame {
            id: AppId(id),
            total_hours,
            last_played: None,
        };
        Signals {
            read_at: Timestamp(0),
            library: vec![
                owned(1, 50.0),
                owned(2, 30.0),
                owned(5, 0.5),
                owned(80, 10.0),
            ],
            borrowed: vec![],
            recent: vec![RecentGame {
                id: AppId(90),
                total_hours: 4.0,
                recent_hours: 1.0,
            }],
            wishlist: vec![AppId(3)],
            followed: vec![AppId(2), AppId(4)],
        }
    }

    fn off_catalog() -> OffCatalogGames {
        OffCatalogGames::from([(
            AppId(80),
            OffCatalogGame {
                tags: vec![GameTag {
                    id: TagId(1),
                    weight: 1.0,
                }],
                name: Some("Skyrim".to_string()),
            },
        )])
    }

    fn votes() -> Votes {
        Votes::from([(
            AppId(6),
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-01".to_string(),
                bet: false,
                genre_ok: false,
                at: None,
            },
        )])
    }

    fn corrections() -> Corrections {
        Corrections {
            game_multipliers: BTreeMap::from([(AppId(2), 0.5)]),
            hidden_games: BTreeSet::from([AppId(3)]),
            ..Corrections::default()
        }
    }

    struct Fixture {
        catalog: Catalog,
        signals: Signals,
        votes: Votes,
        corrections: Corrections,
        families: Families,
        off_catalog: OffCatalogGames,
        config: Config,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture {
                catalog: catalog(),
                signals: signals(),
                votes: votes(),
                corrections: corrections(),
                families: Families::default(),
                off_catalog: off_catalog(),
                config: Config::default(),
            }
        }

        fn inputs(&self) -> SearchInputs<'_> {
            SearchInputs {
                catalog: &self.catalog,
                game_vectors: &[],
                signals: &self.signals,
                votes: &self.votes,
                corrections: &self.corrections,
                families: &self.families,
                off_catalog: &self.off_catalog,
                config: &self.config,
            }
        }
    }

    fn library() -> LibraryView {
        library_view(&Fixture::new().inputs())
    }

    fn find(view: &LibraryView, id: u32) -> &LibraryGameView {
        view.games.iter().find(|game| game.id == AppId(id)).unwrap()
    }

    #[test]
    fn the_library_has_every_game_of_the_signals_but_not_the_votes() {
        let view = library();

        let ids: Vec<u32> = view.games.iter().map(|game| game.id.0).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 80, 90]);
        assert_eq!(view.min_hours, 2.0);
        assert_eq!(view.wishlist_hours, 5.0);
        assert_eq!(view.followed_hours, 3.0);
        assert_eq!(view.dampen, 0.5);
    }

    #[test]
    fn a_game_says_where_it_comes_from() {
        let view = library();

        let aseprite = find(&view, 2);
        assert_eq!(aseprite.owned_hours, Some(30.0));
        assert!(aseprite.followed && !aseprite.wishlist && !aseprite.recent);
        assert_eq!(
            (aseprite.hours, aseprite.source),
            (30.0, HoursSource::Played)
        );
        let portal = find(&view, 3);
        assert_eq!(portal.owned_hours, None);
        assert!(portal.wishlist);
        assert_eq!((portal.hours, portal.source), (5.0, HoursSource::Wishlist));
        let borrowed = find(&view, 90);
        assert_eq!(borrowed.owned_hours, None);
        assert!(borrowed.recent);
        assert_eq!(borrowed.hours, 6.0);
    }

    #[test]
    fn a_game_played_but_not_owned_is_borrowed() {
        let mut fixture = Fixture::new();
        fixture.signals.borrowed = vec![LibraryGame {
            id: AppId(4),
            total_hours: 12.0,
            last_played: None,
        }];

        let view = library_view(&fixture.inputs());

        let game = find(&view, 4);
        assert!(game.borrowed && game.followed);
        assert_eq!(game.owned_hours, None);
        assert_eq!((game.hours, game.source), (12.0, HoursSource::Played));
        assert!(!find(&view, 1).borrowed);
    }

    #[test]
    fn the_view_says_which_families_are_automatic() {
        let mut fixture = Fixture::new();
        fixture.families = Families {
            families: vec![
                Family {
                    name: "mysteries".to_string(),
                    games: vec![AppId(1)],
                    automatic: Some("2026-11-10".to_string()),
                },
                Family {
                    name: "space".to_string(),
                    games: vec![AppId(1)],
                    automatic: None,
                },
            ],
            held_out: Vec::new(),
        };

        let view = library_view(&fixture.inputs());

        assert_eq!(view.automatic_families, ["mysteries"]);
    }

    #[test]
    fn a_game_says_its_families_and_if_it_is_held_out() {
        let mut fixture = Fixture::new();
        fixture.families = Families {
            families: vec![
                Family {
                    name: "mysteries".to_string(),
                    games: vec![AppId(1), AppId(2)],
                    automatic: None,
                },
                Family {
                    name: "space".to_string(),
                    games: vec![AppId(1)],
                    automatic: None,
                },
            ],
            held_out: vec![AppId(5)],
        };
        let view = library_view(&fixture.inputs());

        assert_eq!(view.families, ["mysteries", "space"]);
        assert_eq!(find(&view, 1).families, ["mysteries", "space"]);
        assert_eq!(find(&view, 2).families, ["mysteries"]);
        assert!(find(&view, 3).families.is_empty());
        assert!(find(&view, 5).held_out && !find(&view, 1).held_out);
    }

    #[test]
    fn the_names_come_from_the_catalog_then_from_outside_it() {
        let view = library();

        assert_eq!(find(&view, 1).name, "Outer Wilds");
        assert_eq!(find(&view, 80).name, "Skyrim");
        assert_eq!(find(&view, 90).name, "#90");
    }

    #[test]
    fn the_games_that_count_share_the_taste() {
        let view = library();

        let celeste = find(&view, 5);
        assert!(!celeste.counts);
        assert_eq!(celeste.share, 0.0);
        assert!(celeste.tags.is_empty());
        assert!(!find(&view, 90).counts);
        assert!(find(&view, 80).counts);
        let total: f64 = view.games.iter().map(|game| game.share).sum();
        let would_try = 1.0 - total;
        assert!(would_try > 0.0 && would_try < 0.3, "{would_try}");
        assert!(find(&view, 1).share > find(&view, 2).share);
    }

    #[test]
    fn a_game_brings_its_three_heaviest_tags() {
        let view = library();

        assert_eq!(
            find(&view, 1).tags,
            ["Exploration", "Puzzle", "Pixel Graphics"]
        );
        assert_eq!(find(&view, 80).tags, ["Exploration"]);
    }

    #[test]
    fn a_game_shows_its_corrections() {
        let view = library();

        assert_eq!(
            find(&view, 2).correction,
            GameCorrection {
                multiplier: Some(0.5),
                hidden: false
            }
        );
        assert!(find(&view, 3).correction.hidden);
        assert_eq!(find(&view, 1).correction, GameCorrection::default());
    }

    #[test]
    fn a_game_at_zero_does_not_count() {
        let mut fixture = Fixture::new();
        fixture.corrections.game_multipliers.insert(AppId(1), 0.0);

        let view = library_view(&fixture.inputs());

        assert!(!find(&view, 1).counts);
        assert_eq!(find(&view, 1).share, 0.0);
    }

    fn correct(
        fixture: &Fixture,
        path: &Path,
        id: u32,
        multiplier: Option<f64>,
        hidden: bool,
    ) -> Result<Corrections, GameCorrectionError> {
        corrected_game(
            path,
            AppId(id),
            GameCorrection { multiplier, hidden },
            &fixture.inputs(),
        )
    }

    #[test]
    fn a_correction_is_written_with_the_name_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let mut fixture = Fixture::new();
        fixture.corrections = Corrections::default();

        fixture.corrections = correct(&fixture, &path, 1, Some(0.5), false).unwrap();

        assert_eq!(fixture.corrections.game_multipliers[&AppId(1)], 0.5);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("1 = 0.5 # Outer Wilds"), "{text}");

        fixture.corrections = correct(&fixture, &path, 3, None, true).unwrap();
        assert!(fixture.corrections.hidden_games.contains(&AppId(3)));

        fixture.corrections = correct(&fixture, &path, 1, None, false).unwrap();
        fixture.corrections = correct(&fixture, &path, 3, None, false).unwrap();
        assert_eq!(fixture.corrections, Corrections::default());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("Outer Wilds"), "{text}");
    }

    #[test]
    fn a_multiplier_is_refused_where_it_would_have_no_effect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let fixture = Fixture::new();

        let below = correct(&fixture, &path, 5, Some(0.5), false).unwrap_err();
        assert_eq!(
            below.to_string(),
            "Celeste (5) has 0.50 effective hours, below the threshold of 2 hours: dampening it would have no effect"
        );
        let outside = correct(&fixture, &path, 999, Some(0.5), false).unwrap_err();
        assert!(matches!(outside, GameCorrectionError::NotInSignals { .. }));
        let above_one = correct(&fixture, &path, 1, Some(2.0), false).unwrap_err();
        assert!(matches!(
            above_one,
            GameCorrectionError::FactorOutOfRange { factor: 2.0 }
        ));
        assert!(!path.exists());
    }

    #[test]
    fn a_game_below_the_threshold_can_still_be_hidden_or_lose_its_multiplier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let mut fixture = Fixture::new();
        fixture.corrections.game_multipliers.insert(AppId(5), 0.25);
        std::fs::write(&path, "[game_multipliers]\n5 = 0.25\n").unwrap();

        let kept = correct(&fixture, &path, 5, Some(0.25), true).unwrap();
        assert!(kept.hidden_games.contains(&AppId(5)));
        let removed = correct(&fixture, &path, 5, None, false).unwrap();
        assert!(!removed.game_multipliers.contains_key(&AppId(5)));
    }

    #[test]
    fn the_same_correction_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let fixture = Fixture::new();

        let same = correct(&fixture, &path, 2, Some(0.5), false).unwrap();

        assert_eq!(same, fixture.corrections);
        assert!(!path.exists());
    }

    #[test]
    fn the_correction_of_a_game_reads_both_sections() {
        let corrections = corrections();

        assert_eq!(
            game_correction(AppId(2), &corrections).multiplier,
            Some(0.5)
        );
        assert!(game_correction(AppId(3), &corrections).hidden);
        assert_eq!(
            game_correction(AppId(7), &corrections),
            GameCorrection::default()
        );
    }

    #[test]
    fn a_game_played_elsewhere_is_in_the_library_with_the_fixed_hours() {
        let mut fixture = Fixture::new();
        fixture.corrections.played_elsewhere.insert(AppId(7));

        let view = library_view(&fixture.inputs());

        let game = find(&view, 7);
        assert!(game.elsewhere && game.elsewhere_only);
        assert_eq!((game.hours, game.source), (20.0, HoursSource::Elsewhere));
        assert_eq!(game.owned_hours, None);
        assert_eq!(view.elsewhere_hours, 20.0);
        assert!(!find(&view, 1).elsewhere);
    }

    #[test]
    fn a_game_played_elsewhere_that_is_also_in_the_signals_is_not_only_elsewhere() {
        let mut fixture = Fixture::new();
        fixture
            .corrections
            .played_elsewhere
            .extend([AppId(1), AppId(3), AppId(4), AppId(90)]);

        let view = library_view(&fixture.inputs());

        for id in [1, 3, 4, 90] {
            let game = find(&view, id);
            assert!(game.elsewhere && !game.elsewhere_only, "{id}");
        }
        assert_eq!(
            (find(&view, 1).hours, find(&view, 1).source),
            (50.0, HoursSource::Played)
        );
        assert_eq!(
            (find(&view, 3).hours, find(&view, 3).source),
            (20.0, HoursSource::Elsewhere)
        );
    }

    #[test]
    fn a_game_played_elsewhere_can_be_halved_and_goes_in_a_family() {
        let mut fixture = Fixture::new();
        fixture.corrections.played_elsewhere.insert(AppId(7));
        fixture.families = Families {
            families: vec![Family {
                name: "roguelikes".to_string(),
                games: vec![AppId(7)],
                automatic: None,
            }],
            held_out: vec![],
        };

        assert!(check_game_multiplier(AppId(7), 0.5, &fixture.inputs()).is_ok());
        assert!(matches!(
            check_game_multiplier(AppId(8), 0.5, &fixture.inputs()),
            Err(GameCorrectionError::NotInSignals { .. })
        ));
        let view = library_view(&fixture.inputs());
        assert_eq!(find(&view, 7).families, ["roguelikes"]);
    }

    #[test]
    fn a_game_played_elsewhere_is_added_removed_and_added_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        std::fs::write(&path, "# mine\n\n[game_multipliers]\n6 = 0.5\n").unwrap();
        let mut corrections = Corrections {
            game_multipliers: BTreeMap::from([(AppId(6), 0.5)]),
            ..Corrections::default()
        };

        corrections =
            changed_played_elsewhere(&path, AppId(6), true, &corrections, Some("Balatro")).unwrap();
        assert!(corrections.played_elsewhere.contains(&AppId(6)));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("6 = true # Balatro"), "{text}");
        assert!(text.starts_with("# mine"));

        corrections =
            changed_played_elsewhere(&path, AppId(6), false, &corrections, Some("Balatro"))
                .unwrap();
        assert!(corrections.played_elsewhere.is_empty());
        assert_eq!(corrections.game_multipliers[&AppId(6)], 0.5);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("6 = 0.5") && !text.contains("6 = true"),
            "{text}"
        );

        corrections =
            changed_played_elsewhere(&path, AppId(6), true, &corrections, Some("Balatro")).unwrap();
        assert!(corrections.played_elsewhere.contains(&AppId(6)));
        assert_eq!(corrections.game_multipliers[&AppId(6)], 0.5);
    }

    #[test]
    fn a_game_already_in_the_state_asked_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let corrections = Corrections {
            played_elsewhere: BTreeSet::from([AppId(6)]),
            ..Corrections::default()
        };

        let same = changed_played_elsewhere(&path, AppId(6), true, &corrections, None).unwrap();
        let none = changed_played_elsewhere(&path, AppId(7), false, &corrections, None).unwrap();

        assert_eq!(same, corrections);
        assert_eq!(none, corrections);
        assert!(!path.exists());
    }

    #[test]
    fn an_invalid_appid_or_a_file_that_cannot_be_written_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        let corrections = Corrections::default();

        let zero = changed_played_elsewhere(&path, AppId(0), true, &corrections, None).unwrap_err();
        assert_eq!(zero.to_string(), "0 is not a valid Steam appid");

        let missing = dir.path().join("missing").join("corrections.toml");
        let unwritable =
            changed_played_elsewhere(&missing, AppId(6), true, &corrections, None).unwrap_err();
        assert_eq!(
            unwritable.to_string(),
            format!(
                "could not save the game played elsewhere in {}",
                missing.display()
            )
        );
        assert!(!path.exists());
    }

    #[test]
    fn the_ids_of_the_library_include_the_games_played_elsewhere() {
        let mut fixture = Fixture::new();
        fixture.corrections.played_elsewhere.insert(AppId(7));

        let ids = library_ids(&fixture.inputs());

        assert!([7, 1, 90].iter().all(|&id| ids.contains(&AppId(id))));
        assert!(!ids.contains(&AppId(8)));
    }
}
