use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;

use crate::{
    catalog::Catalog,
    creators::LikedCreators,
    game::Game,
    ids::{AppId, TagId},
    votes::{VoteKind, Votes},
};

const MIN_TAG_GAMES: usize = 8;
pub const MIN_LABELS: usize = 10;
const STEPS: usize = 400;
const LEARNING_RATE: f64 = 0.5;
const L2: f64 = 0.1;
const FOLDS: usize = 10;

#[derive(Debug, Clone)]
struct TrainingRow<'a> {
    game: &'a Game,
    label: f64,
}

#[derive(Debug, Clone)]
pub struct Standardization {
    pub means: Vec<f64>,
    pub deviations: Vec<f64>,
}

impl Standardization {
    pub fn apply(&self, features: &[f64]) -> Vec<f64> {
        features
            .iter()
            .zip(&self.means)
            .zip(&self.deviations)
            .map(|((x, mean), deviation)| (x - mean) / deviation)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct OpinionModel {
    pub tags: Vec<TagId>,
    pub liked: LikedCreators,
    pub standardization: Standardization,
    pub weights: Vec<f64>,
}

impl OpinionModel {
    pub fn opinion(&self, game: &Game) -> f64 {
        let features = features(game, &self.tags, &self.liked);
        let standardized_features = self.standardization.apply(&features);

        dot(&standardized_features, &self.weights)
    }
}

fn training_rows<'a>(votes: &Votes, catalog: &'a Catalog) -> Vec<TrainingRow<'a>> {
    votes
        .iter()
        .filter_map(|(id, vote)| {
            let label = match vote.kind {
                VoteKind::WouldTry => 1.0,
                VoteKind::NotForMe => 0.0,
                _ => return None,
            };
            let game = catalog.game(*id)?;
            Some(TrainingRow { game, label })
        })
        .collect()
}

fn used_tags(training_rows: &[TrainingRow]) -> Vec<TagId> {
    let mut seen: BTreeMap<TagId, usize> = BTreeMap::new();

    for row in training_rows {
        for tag in &row.game.tags {
            *seen.entry(tag.id).or_insert(0) += 1;
        }
    }

    seen.into_iter()
        .filter(|&(_, count)| count >= MIN_TAG_GAMES)
        .map(|(id, _)| id)
        .collect()
}

fn features(game: &Game, tag_ids: &[TagId], liked: &LikedCreators) -> Vec<f64> {
    let positive_share = if game.reviews.total == 0 {
        0.0
    } else {
        game.reviews.positive as f64 / game.reviews.total as f64
    };

    let mut features = vec![0.0; 5 + tag_ids.len()];
    features[0] = (1.0 + game.reviews.total as f64).log10();
    features[1] = positive_share;
    features[2] = (1.0 + game.price.original_cents as f64).log10();
    if liked.has_liked_developer(game) {
        features[3] = 1.0;
    }
    if liked.has_liked_franchise(game) {
        features[4] = 1.0;
    }

    for tag in &game.tags {
        if let Ok(i) = tag_ids.binary_search(&tag.id) {
            features[5 + i] = tag.weight;
        }
    }

    features
}

fn standardization(features: &[Vec<f64>]) -> Standardization {
    let mut means: Vec<f64> = Vec::new();
    let mut deviations: Vec<f64> = Vec::new();

    let n = features.len() as f64;

    for j in 0..features[0].len() {
        let mean = features.iter().map(|row| row[j]).sum::<f64>() / n;
        means.push(mean);

        let deviation = (features
            .iter()
            .map(|row| (row[j] - mean).powi(2))
            .sum::<f64>()
            / n)
            .sqrt();
        deviations.push(if deviation == 0.0 { 1.0 } else { deviation });
    }

    Standardization { means, deviations }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn train(rows: &[Vec<f64>], labels: &[f64]) -> Vec<f64> {
    let mut w: Vec<f64> = vec![0.0; rows[0].len()];
    let mut b: f64 = 0.0;
    let n = rows.len() as f64;

    for _ in 0..STEPS {
        let mut grad = vec![0.0; w.len()];
        let mut grad_b = 0.0;

        for (row, label) in rows.iter().zip(labels) {
            let score = b + dot(row, &w);
            let prediction = sigmoid(score);
            let error = prediction - label;

            for (g, z) in grad.iter_mut().zip(row) {
                *g += error * z;
            }
            grad_b += error;
        }

        for (w_j, grad_j) in w.iter_mut().zip(&grad) {
            *w_j -= LEARNING_RATE * (grad_j / n + L2 * *w_j);
        }
        b -= LEARNING_RATE * grad_b / n;
    }

    w
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labels {
    pub would_try: usize,
    pub not_for_me: usize,
}

impl Labels {
    pub fn enough(&self) -> bool {
        self.would_try >= MIN_LABELS && self.not_for_me >= MIN_LABELS
    }
}

pub fn labels(votes: &Votes, catalog: &Catalog) -> Labels {
    let training_rows = training_rows(votes, catalog);
    let would_try = training_rows.iter().filter(|row| row.label == 1.0).count();

    Labels {
        would_try,
        not_for_me: training_rows.len() - would_try,
    }
}

pub fn opinion_model(
    votes: &Votes,
    catalog: &Catalog,
    liked: &LikedCreators,
) -> Option<OpinionModel> {
    model(&training_rows(votes, catalog), liked)
}

pub fn held_out_opinions(
    votes: &Votes,
    catalog: &Catalog,
    liked: &LikedCreators,
) -> HashMap<AppId, f64> {
    let training_rows = training_rows(votes, catalog);
    let Some(full_model) = model(&training_rows, liked) else {
        return HashMap::new();
    };

    let mut opinions: HashMap<AppId, f64> = votes
        .keys()
        .filter_map(|&id| catalog.game(id))
        .map(|game| (game.id, full_model.opinion(game)))
        .collect();

    let folds: Vec<Vec<(AppId, Option<f64>)>> = (0..FOLDS)
        .into_par_iter()
        .map(|fold| {
            let kept: Vec<TrainingRow> = training_rows
                .iter()
                .enumerate()
                .filter(|(i, _)| i % FOLDS != fold)
                .map(|(_, row)| row.clone())
                .collect();
            let fold_model = model(&kept, liked);

            training_rows
                .iter()
                .enumerate()
                .filter(|(i, _)| i % FOLDS == fold)
                .map(|(_, row)| {
                    (
                        row.game.id,
                        fold_model.as_ref().map(|model| model.opinion(row.game)),
                    )
                })
                .collect()
        })
        .collect();

    for (id, opinion) in folds.into_iter().flatten() {
        match opinion {
            Some(opinion) => opinions.insert(id, opinion),
            None => opinions.remove(&id),
        };
    }

    opinions
}

fn model(training_rows: &[TrainingRow], liked: &LikedCreators) -> Option<OpinionModel> {
    let positive_labels = training_rows.iter().filter(|row| row.label == 1.0).count();
    let negative_labels = training_rows.len() - positive_labels;
    if positive_labels < MIN_LABELS || negative_labels < MIN_LABELS {
        return None;
    }

    let tags = used_tags(training_rows);
    let features = training_rows
        .iter()
        .map(|row| features(row.game, &tags, liked))
        .collect::<Vec<_>>();
    let standardization = standardization(&features);
    let standardized_features = features
        .iter()
        .map(|feature| standardization.apply(feature))
        .collect::<Vec<_>>();
    let labels = training_rows
        .iter()
        .map(|row| row.label)
        .collect::<Vec<_>>();

    let weights = train(&standardized_features, &labels);

    Some(OpinionModel {
        tags,
        liked: liked.clone(),
        standardization,
        weights,
    })
}

pub fn opinions(opinion_model: &OpinionModel, catalog: &Catalog) -> Vec<f64> {
    catalog
        .games()
        .par_iter()
        .map(|game| opinion_model.opinion(game))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        catalog::Freshness,
        currency::Currency,
        game::{GameTag, Price, Reviews},
        ids::AppId,
        region::Region,
        timestamp::Timestamp,
        votes::Vote,
    };

    fn game(id: u32) -> Game {
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
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn vote(kind: VoteKind) -> Vote {
        Vote {
            kind,
            date: "2026-10-01".to_string(),
            bet: false,
            genre_ok: false,
            at: None,
        }
    }

    #[test]
    fn the_labels_count_only_the_voted_games_of_the_catalog() {
        let catalog = Catalog::new(
            vec![game(1), game(2), game(3)],
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let votes = Votes::from([
            (AppId(1), vote(VoteKind::WouldTry)),
            (AppId(2), vote(VoteKind::NotForMe)),
            (AppId(3), vote(VoteKind::Later)),
            (AppId(4), vote(VoteKind::WouldTry)),
        ]);

        let labels = labels(&votes, &catalog);

        assert_eq!(
            labels,
            Labels {
                would_try: 1,
                not_for_me: 1
            }
        );
        assert!(!labels.enough());
    }

    #[test]
    fn ten_labels_of_each_kind_are_enough() {
        let enough = |would_try, not_for_me| {
            Labels {
                would_try,
                not_for_me,
            }
            .enough()
        };

        assert!(enough(10, 10));
        assert!(!enough(10, 9));
        assert!(!enough(9, 30));
    }

    fn tagged_catalog(count: u32) -> Catalog {
        let games = (1..=count)
            .map(|id| Game {
                tags: vec![GameTag {
                    id: TagId(id % 3),
                    weight: 1.0,
                }],
                reviews: Reviews {
                    total: 100 * id,
                    positive: 90 * id - id % 7,
                },
                ..game(id)
            })
            .collect();

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

    fn alternate_votes(count: u32) -> Votes {
        (1..=count)
            .map(|id| {
                let kind = if id % 2 == 0 || id % 3 == 0 {
                    VoteKind::WouldTry
                } else {
                    VoteKind::NotForMe
                };
                (AppId(id), vote(kind))
            })
            .collect()
    }

    #[test]
    fn a_held_out_opinion_does_not_depend_on_the_vote_of_its_game() {
        let catalog = tagged_catalog(40);
        let votes = alternate_votes(40);
        let mut changed = votes.clone();
        changed.insert(AppId(12), vote(VoteKind::NotForMe));

        let before = held_out_opinions(&votes, &catalog, &LikedCreators::default());
        let after = held_out_opinions(&changed, &catalog, &LikedCreators::default());

        assert_eq!(before.len(), 40);
        assert_eq!(before[&AppId(12)], after[&AppId(12)]);
        assert_ne!(before[&AppId(13)], after[&AppId(13)]);
    }

    #[test]
    fn the_held_out_opinions_are_the_same_at_every_call() {
        let catalog = tagged_catalog(40);
        let votes = alternate_votes(40);

        assert_eq!(
            held_out_opinions(&votes, &catalog, &LikedCreators::default()),
            held_out_opinions(&votes, &catalog, &LikedCreators::default())
        );
    }

    #[test]
    fn a_vote_that_is_not_a_label_takes_the_opinion_of_the_whole_model() {
        let catalog = tagged_catalog(41);
        let mut votes = alternate_votes(40);
        votes.insert(AppId(41), vote(VoteKind::Later));

        let opinions = held_out_opinions(&votes, &catalog, &LikedCreators::default());
        let model = opinion_model(&votes, &catalog, &LikedCreators::default()).unwrap();

        assert_eq!(
            opinions[&AppId(41)],
            model.opinion(catalog.game(AppId(41)).unwrap())
        );
    }

    fn liked_developer(name: &str, games: &[u32]) -> LikedCreators {
        LikedCreators {
            games: games.iter().map(|&id| AppId(id)).collect(),
            developers: BTreeMap::from([(name.to_string(), games.len())]),
            franchises: BTreeMap::new(),
        }
    }

    #[test]
    fn a_new_game_of_a_liked_developer_gets_a_higher_opinion() {
        let developer = |id: u32| {
            if id <= 18 || id == 39 || id == 50 {
                "Liked Studio"
            } else {
                "Other Studio"
            }
        };
        let games = (1..=51)
            .map(|id| Game {
                tags: vec![GameTag {
                    id: TagId(1),
                    weight: 1.0,
                }],
                developers: vec![developer(id).to_string()],
                ..game(id)
            })
            .collect();
        let catalog = Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let votes: Votes = (1..=40)
            .map(|id| {
                let kind = if id <= 20 {
                    VoteKind::WouldTry
                } else {
                    VoteKind::NotForMe
                };
                (AppId(id), vote(kind))
            })
            .collect();

        let model =
            opinion_model(&votes, &catalog, &liked_developer("liked studio", &[100])).unwrap();
        let opinion = |id| model.opinion(catalog.game(AppId(id)).unwrap());

        assert!(model.weights[3] > 0.0);
        assert!(opinion(50) > opinion(51));
    }

    #[test]
    fn without_signals_the_opinion_is_the_one_without_the_creator_features() {
        let tagged = tagged_catalog(41);
        let games = tagged
            .games()
            .iter()
            .map(|game| Game {
                developers: vec![format!("studio {}", game.id.0 % 4)],
                franchises: vec![format!("saga {}", game.id.0 % 5)],
                ..game.clone()
            })
            .collect();
        let catalog = Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        let votes = alternate_votes(40);
        let liked = LikedCreators::default();

        let model = opinion_model(&votes, &catalog, &liked).unwrap();

        let rows = training_rows(&votes, &catalog);
        let without_creators = |game: &Game| {
            let mut features = features(game, &model.tags, &liked);
            features.drain(3..5);
            features
        };
        let old_features: Vec<Vec<f64>> =
            rows.iter().map(|row| without_creators(row.game)).collect();
        let old_standardization = standardization(&old_features);
        let old_weights = train(
            &old_features
                .iter()
                .map(|features| old_standardization.apply(features))
                .collect::<Vec<_>>(),
            &rows.iter().map(|row| row.label).collect::<Vec<_>>(),
        );

        assert_eq!(&model.weights[3..5], &[0.0, 0.0]);
        for game in catalog.games() {
            let old_opinion = dot(
                &old_standardization.apply(&without_creators(game)),
                &old_weights,
            );
            assert!((model.opinion(game) - old_opinion).abs() < 1e-12);
        }
    }

    #[test]
    fn without_enough_labels_there_are_no_held_out_opinions() {
        let catalog = tagged_catalog(10);

        assert!(
            held_out_opinions(&alternate_votes(10), &catalog, &LikedCreators::default()).is_empty()
        );
    }
}
