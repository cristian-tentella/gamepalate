use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;

use crate::{
    catalog::Catalog,
    creators::LikedCreators,
    game::Game,
    ids::{AppId, TagId},
    votes::Votes,
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

#[derive(Debug, Clone)]
pub struct OpinionModel {
    pub tags: Vec<TagId>,
    pub liked: LikedCreators,
    pub standardization: Standardization,
    pub weights: Vec<f64>,
}

impl OpinionModel {
    pub fn opinion(&self, game: &Game) -> f64 {
        self.opinion_with(game, &mut Vec::new())
    }

    // The terms of the standardized row times the weights, place by place, with 0 where the
    // game has no feature: the sum of the dense row, in its order, without writing it.
    fn opinion_with(&self, game: &Game, features: &mut Vec<(usize, f64)>) -> f64 {
        features_into(game, &self.tags, &self.liked, features);
        let Standardization { means, deviations } = &self.standardization;
        let mut present = features.iter().peekable();

        means
            .iter()
            .zip(deviations)
            .zip(&self.weights)
            .enumerate()
            .map(|(j, ((mean, deviation), w))| {
                let x = present
                    .next_if(|&&(place, _)| place == j)
                    .map_or(0.0, |&(_, x)| x);
                (x - mean) / deviation * w
            })
            .sum()
    }
}

fn training_rows<'a>(votes: &Votes, catalog: &'a Catalog) -> Vec<TrainingRow<'a>> {
    votes
        .iter()
        .filter_map(|(id, vote)| {
            let label = vote.kind.label()?;
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

fn columns(tag_ids: &[TagId]) -> usize {
    5 + tag_ids.len()
}

// The features of a game without its zeros, as (place, value) in the order of the places:
// a game has few of the tags.
fn features(game: &Game, tag_ids: &[TagId], liked: &LikedCreators) -> Vec<(usize, f64)> {
    let mut features = Vec::new();
    features_into(game, tag_ids, liked, &mut features);
    features
}

fn features_into(
    game: &Game,
    tag_ids: &[TagId],
    liked: &LikedCreators,
    features: &mut Vec<(usize, f64)>,
) {
    let positive_share = if game.reviews.total == 0 {
        0.0
    } else {
        game.reviews.positive as f64 / game.reviews.total as f64
    };
    let first = [
        (1.0 + game.reviews.total as f64).log10(),
        positive_share,
        (1.0 + game.price.original_cents as f64).log10(),
        if liked.has_liked_developer(game) {
            1.0
        } else {
            0.0
        },
        if liked.has_liked_franchise(game) {
            1.0
        } else {
            0.0
        },
    ];

    features.clear();
    features.extend(first.into_iter().enumerate().filter(|&(_, x)| x != 0.0));
    let tags_start = features.len();
    // From the last tag back, so a repeated tag keeps its last weight, even a 0
    for tag in game.tags.iter().rev() {
        if let Ok(i) = tag_ids.binary_search(&tag.id)
            && !features[tags_start..]
                .iter()
                .any(|&(place, _)| place == 5 + i)
        {
            features.push((5 + i, tag.weight));
        }
    }
    features.retain(|&(_, x)| x != 0.0);
    features[tags_start..].sort_unstable_by_key(|&(place, _)| place);
}

// The sums of the dense rows, in their order: the zeros add nothing to the means, but every
// row gives its (x - mean)² to every deviation, with x = 0 where it has no feature.
fn standardization(rows: &[Vec<(usize, f64)>], columns: usize) -> Standardization {
    let n = rows.len() as f64;

    let mut sums = vec![0.0; columns];
    for row in rows {
        for &(j, x) in row {
            sums[j] += x;
        }
    }
    let means: Vec<f64> = sums.iter().map(|sum| sum / n).collect();

    let mut squares = vec![0.0; columns];
    for row in rows {
        let mut present = row.iter().peekable();
        for (j, (square, mean)) in squares.iter_mut().zip(&means).enumerate() {
            let x = present
                .next_if(|&&(place, _)| place == j)
                .map_or(0.0, |&(_, x)| x);
            *square += (x - mean).powi(2);
        }
    }
    let deviations = squares
        .iter()
        .map(|square| {
            let deviation = (square / n).sqrt();
            if deviation == 0.0 { 1.0 } else { deviation }
        })
        .collect();

    Standardization { means, deviations }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

// Gradient descent on the standardized features, (x - mean) / deviation, without
// writing them: on sparse rows each step only touches the features a game has.
fn train(
    rows: &[Vec<(usize, f64)>],
    standardization: &Standardization,
    labels: &[f64],
) -> Vec<f64> {
    let Standardization { means, deviations } = standardization;
    let mut w: Vec<f64> = vec![0.0; means.len()];
    let mut b: f64 = 0.0;
    let n = rows.len() as f64;

    for _ in 0..STEPS {
        let scaled: Vec<f64> = w.iter().zip(deviations).map(|(w, d)| w / d).collect();
        let offset = b - dot(means, &scaled);
        let mut sums = vec![0.0; w.len()];
        let mut grad_b = 0.0;

        for (row, label) in rows.iter().zip(labels) {
            let score = offset + row.iter().map(|&(j, x)| x * scaled[j]).sum::<f64>();
            let error = sigmoid(score) - label;

            for &(j, x) in row {
                sums[j] += error * x;
            }
            grad_b += error;
        }

        for (j, w_j) in w.iter_mut().enumerate() {
            let grad_j = (sums[j] - means[j] * grad_b) / deviations[j];
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
    let rows = training_rows
        .iter()
        .map(|row| features(row.game, &tags, liked))
        .collect::<Vec<_>>();
    let standardization = standardization(&rows, columns(&tags));
    let labels = training_rows
        .iter()
        .map(|row| row.label)
        .collect::<Vec<_>>();

    let weights = train(&rows, &standardization, &labels);

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
        .map_init(Vec::new, |features, game| {
            opinion_model.opinion_with(game, features)
        })
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
        votes::{Vote, VoteKind},
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
        Vote::new(kind, "2026-10-01")
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
                .map(|features| sparse(features))
                .collect::<Vec<_>>(),
            &old_standardization,
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

    // The features as they were, every place written out, and what read them: the reference
    // for the opinion on pairs
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

    fn sparse(features: &[f64]) -> Vec<(usize, f64)> {
        features
            .iter()
            .enumerate()
            .filter(|&(_, &x)| x != 0.0)
            .map(|(j, &x)| (j, x))
            .collect()
    }

    impl Standardization {
        fn apply(&self, features: &[f64]) -> Vec<f64> {
            features
                .iter()
                .zip(&self.means)
                .zip(&self.deviations)
                .map(|((x, mean), deviation)| (x - mean) / deviation)
                .collect()
        }
    }

    fn dense_model(training_rows: &[TrainingRow], liked: &LikedCreators) -> Option<OpinionModel> {
        let positive_labels = training_rows.iter().filter(|row| row.label == 1.0).count();
        if positive_labels < MIN_LABELS || training_rows.len() - positive_labels < MIN_LABELS {
            return None;
        }
        let tags = used_tags(training_rows);
        let features: Vec<Vec<f64>> = training_rows
            .iter()
            .map(|row| features(row.game, &tags, liked))
            .collect();
        let standardization = standardization(&features);
        let rows: Vec<_> = features.iter().map(|row| sparse(row)).collect();
        let labels: Vec<f64> = training_rows.iter().map(|row| row.label).collect();
        let weights = train(&rows, &standardization, &labels);

        Some(OpinionModel {
            tags,
            liked: liked.clone(),
            standardization,
            weights,
        })
    }

    fn dense_opinion(model: &OpinionModel, game: &Game) -> f64 {
        let features = features(game, &model.tags, &model.liked);
        dot(&model.standardization.apply(&features), &model.weights)
    }

    fn dense_held_out_opinions(
        votes: &Votes,
        catalog: &Catalog,
        liked: &LikedCreators,
    ) -> HashMap<AppId, f64> {
        let training_rows = training_rows(votes, catalog);
        let Some(full_model) = dense_model(&training_rows, liked) else {
            return HashMap::new();
        };
        let mut opinions: HashMap<AppId, f64> = votes
            .keys()
            .filter_map(|&id| catalog.game(id))
            .map(|game| (game.id, dense_opinion(&full_model, game)))
            .collect();
        for fold in 0..FOLDS {
            let kept: Vec<TrainingRow> = training_rows
                .iter()
                .enumerate()
                .filter(|(i, _)| i % FOLDS != fold)
                .map(|(_, row)| row.clone())
                .collect();
            let fold_model = dense_model(&kept, liked);
            for (_, row) in training_rows
                .iter()
                .enumerate()
                .filter(|(i, _)| i % FOLDS == fold)
            {
                match &fold_model {
                    Some(model) => opinions.insert(row.game.id, dense_opinion(model, row.game)),
                    None => opinions.remove(&row.game.id),
                };
            }
        }
        opinions
    }

    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|x| x.to_bits()).collect()
    }

    fn bits_by_id(opinions: &HashMap<AppId, f64>) -> BTreeMap<AppId, u64> {
        opinions.iter().map(|(id, x)| (*id, x.to_bits())).collect()
    }

    // Zeros in the first five places, tags out of order, a repeated tag (once with a last
    // weight of 0), tags of fewer than MIN_TAG_GAMES games, liked developers and franchises,
    // and games out of the training without any used tag
    fn uneven_catalog() -> Catalog {
        let tag = |id: u32, weight: f64| GameTag {
            id: TagId(id),
            weight,
        };
        let games = (1..=50)
            .map(|id| {
                let mut tags = vec![
                    tag(9, f64::from(id % 7) / 7.0 + 0.1),
                    tag(
                        2,
                        if id % 6 == 0 {
                            0.0
                        } else {
                            1.0 / f64::from(id)
                        },
                    ),
                ];
                if id % 4 == 0 {
                    tags.push(tag(5, 0.3));
                }
                if id % 5 == 0 {
                    tags.push(tag(9, 0.77));
                }
                if id % 10 == 0 {
                    tags.push(tag(2, 0.0));
                }
                tags.push(tag(100 + id, 0.5));
                if id == 45 {
                    tags.clear();
                }
                if id == 46 {
                    tags = vec![tag(300, 1.0)];
                }
                Game {
                    tags,
                    reviews: if id % 8 == 0 {
                        Reviews {
                            total: 0,
                            positive: 0,
                        }
                    } else {
                        Reviews {
                            total: 37 * id,
                            positive: 30 * id - id % 11,
                        }
                    },
                    price: Price {
                        original_cents: if id % 9 == 0 { 0 } else { 499 + 100 * id },
                        discount: None,
                    },
                    developers: vec![
                        if id % 3 == 0 { "Liked Studio" } else { "Other" }.to_string(),
                    ],
                    franchises: if id % 4 == 1 {
                        vec!["Liked Saga".to_string()]
                    } else {
                        Vec::new()
                    },
                    ..game(id)
                }
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

    fn liked_creators() -> LikedCreators {
        LikedCreators {
            games: [AppId(1000)].into_iter().collect(),
            developers: BTreeMap::from([("liked studio".to_string(), 3)]),
            franchises: BTreeMap::from([("liked saga".to_string(), 3)]),
        }
    }

    #[test]
    fn the_features_on_pairs_are_the_dense_ones_without_zeros() {
        let catalog = uneven_catalog();
        let liked = liked_creators();
        let tags = used_tags(&training_rows(&alternate_votes(40), &catalog));

        for game in catalog.games() {
            assert_eq!(
                super::features(game, &tags, &liked),
                sparse(&features(game, &tags, &liked)),
                "{:?}",
                game.id
            );
        }
        assert!(tags.contains(&TagId(2)) && tags.contains(&TagId(5)) && tags.contains(&TagId(9)));
        assert!(!tags.contains(&TagId(101)));
    }

    #[test]
    fn the_opinion_on_pairs_is_the_dense_one_to_the_bit() {
        let catalog = uneven_catalog();
        let votes = alternate_votes(40);

        for liked in [liked_creators(), LikedCreators::default()] {
            let model = opinion_model(&votes, &catalog, &liked).unwrap();
            let dense = dense_model(&training_rows(&votes, &catalog), &liked).unwrap();

            assert_eq!(
                bits(&model.standardization.means),
                bits(&dense.standardization.means)
            );
            assert_eq!(
                bits(&model.standardization.deviations),
                bits(&dense.standardization.deviations)
            );
            assert_eq!(bits(&model.weights), bits(&dense.weights));
            let dense_opinions: Vec<f64> = catalog
                .games()
                .iter()
                .map(|game| dense_opinion(&dense, game))
                .collect();
            assert_eq!(bits(&opinions(&model, &catalog)), bits(&dense_opinions));
            for game in catalog.games() {
                assert_eq!(
                    model.opinion(game).to_bits(),
                    dense_opinion(&dense, game).to_bits()
                );
            }
        }
    }

    #[test]
    fn the_held_out_opinions_on_pairs_are_the_dense_ones_to_the_bit() {
        let catalog = uneven_catalog();
        let mut votes = alternate_votes(40);
        votes.insert(AppId(45), vote(VoteKind::Later));
        votes.insert(AppId(46), vote(VoteKind::Later));
        let liked = liked_creators();

        let opinions = held_out_opinions(&votes, &catalog, &liked);

        assert_eq!(opinions.len(), 42);
        assert_eq!(
            bits_by_id(&opinions),
            bits_by_id(&dense_held_out_opinions(&votes, &catalog, &liked))
        );
    }

    #[test]
    fn with_too_few_labels_in_a_fold_the_held_out_opinions_are_still_the_dense_ones() {
        let catalog = uneven_catalog();
        let votes: Votes = (1..=22)
            .map(|id| {
                let kind = if id <= 11 {
                    VoteKind::WouldTry
                } else {
                    VoteKind::NotForMe
                };
                (AppId(id), vote(kind))
            })
            .collect();
        let liked = liked_creators();

        let opinions = held_out_opinions(&votes, &catalog, &liked);

        assert_eq!(opinions.len(), 22 - 6);
        assert_eq!(
            bits_by_id(&opinions),
            bits_by_id(&dense_held_out_opinions(&votes, &catalog, &liked))
        );
    }

    // The training as it was, on the standardized rows written out
    fn dense_train(rows: &[Vec<f64>], labels: &[f64]) -> Vec<f64> {
        let mut w = vec![0.0; rows[0].len()];
        let mut b = 0.0;
        let n = rows.len() as f64;
        for _ in 0..STEPS {
            let mut grad = vec![0.0; w.len()];
            let mut grad_b = 0.0;
            for (row, label) in rows.iter().zip(labels) {
                let error = sigmoid(b + dot(row, &w)) - label;
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

    #[test]
    fn the_sparse_training_gives_the_weights_of_the_standardized_rows() {
        let catalog = tagged_catalog(41);
        let votes = alternate_votes(40);
        let liked = LikedCreators::default();
        let rows = training_rows(&votes, &catalog);
        let tags = used_tags(&rows);
        let features: Vec<Vec<f64>> = rows
            .iter()
            .map(|row| features(row.game, &tags, &liked))
            .collect();
        let standardization = standardization(&features);
        let labels: Vec<f64> = rows.iter().map(|row| row.label).collect();

        let sparse_weights = train(
            &features.iter().map(|row| sparse(row)).collect::<Vec<_>>(),
            &standardization,
            &labels,
        );
        let dense_weights = dense_train(
            &features
                .iter()
                .map(|row| standardization.apply(row))
                .collect::<Vec<_>>(),
            &labels,
        );

        assert!(features.iter().flatten().any(|&x| x == 0.0));
        for (sparse, dense) in sparse_weights.iter().zip(&dense_weights) {
            assert!((sparse - dense).abs() < 1e-12);
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
