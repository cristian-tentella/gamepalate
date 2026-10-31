use std::collections::BTreeMap;

use rayon::prelude::*;

use crate::{
    catalog::Catalog,
    game::Game,
    ids::TagId,
    votes::{VoteKind, Votes},
};

const MIN_TAG_GAMES: usize = 8;
pub const MIN_LABELS: usize = 10;
const STEPS: usize = 400;
const LEARNING_RATE: f64 = 0.5;
const L2: f64 = 0.1;

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
    pub standardization: Standardization,
    pub weights: Vec<f64>,
}

impl OpinionModel {
    pub fn opinion(&self, game: &Game) -> f64 {
        let features = features(game, &self.tags);
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

fn features(game: &Game, tag_ids: &[TagId]) -> Vec<f64> {
    let positive_share = if game.reviews.total == 0 {
        0.0
    } else {
        game.reviews.positive as f64 / game.reviews.total as f64
    };

    let mut features = vec![0.0; 3 + tag_ids.len()];
    features[0] = (1.0 + game.reviews.total as f64).log10();
    features[1] = positive_share;
    features[2] = (1.0 + game.price.original_cents as f64).log10();

    for tag in &game.tags {
        if let Ok(i) = tag_ids.binary_search(&tag.id) {
            features[3 + i] = tag.weight;
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

pub fn opinion_model(votes: &Votes, catalog: &Catalog) -> Option<OpinionModel> {
    let training_rows = training_rows(votes, catalog);

    let positive_labels = training_rows.iter().filter(|row| row.label == 1.0).count();
    let negative_labels = training_rows.len() - positive_labels;
    if positive_labels < MIN_LABELS || negative_labels < MIN_LABELS {
        return None;
    }

    let tags = used_tags(&training_rows);
    let features = training_rows
        .iter()
        .map(|row| features(row.game, &tags))
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
        game::{Price, Reviews},
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
        }
    }

    fn vote(kind: VoteKind) -> Vote {
        Vote {
            kind,
            date: "2026-10-01".to_string(),
            bet: false,
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
            (AppId(3), vote(VoteKind::Maybe)),
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
}
