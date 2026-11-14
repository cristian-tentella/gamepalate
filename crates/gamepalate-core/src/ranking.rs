use std::{cmp::Ordering, collections::HashMap};

use crate::{
    affinity::affinities,
    config::RelevanceConfig,
    game_vector::GameVector,
    ids::AppId,
    taste::Taste,
    timestamp::Timestamp,
    votes::{Votes, standing},
};

#[derive(Debug, Clone)]
pub struct RankedGame {
    pub id: AppId,
    pub relevance: f64,
    pub affinity: f64,
}

pub fn relevance(affinity: f64, quality: f64, quality_weight: f64) -> f64 {
    affinity * (1.0 - quality_weight + quality_weight * quality)
}

pub fn ranking_order(a: &RankedGame, b: &RankedGame) -> Ordering {
    b.relevance.total_cmp(&a.relevance).then(a.id.cmp(&b.id))
}

pub fn sort_ranking(ranked_games: &mut [RankedGame]) {
    ranked_games.sort_unstable_by(ranking_order);
}

pub fn ranking(
    game_vectors: &[GameVector],
    taste: &Taste,
    relevance_config: &RelevanceConfig,
) -> Option<Vec<RankedGame>> {
    let affinities = affinities(game_vectors, taste)?;
    let mut ranked_games: Vec<RankedGame> = affinities
        .into_iter()
        .zip(game_vectors)
        .map(|(affinity, game)| RankedGame {
            id: game.id,
            affinity: affinity.affinity,
            relevance: relevance(
                affinity.affinity,
                game.quality,
                relevance_config.quality_weight,
            ),
        })
        .collect();
    sort_ranking(&mut ranked_games);
    Some(ranked_games)
}

pub fn with_votes(
    ranked_games: &[RankedGame],
    opinions: &HashMap<AppId, f64>,
    votes_weight: f64,
) -> Vec<RankedGame> {
    let mut voted: Vec<RankedGame> = ranked_games
        .iter()
        .map(|ranked_game| {
            let opinion = opinions.get(&ranked_game.id).copied().unwrap_or(0.0);
            let mut voted_game = ranked_game.clone();
            voted_game.relevance *= (votes_weight * opinion).exp();
            voted_game
        })
        .collect();
    sort_ranking(&mut voted);
    voted
}

pub fn bets(
    ranked_games: &[RankedGame],
    opinions: &HashMap<AppId, f64>,
    votes: &Votes,
    votes_weight: f64,
    now: Timestamp,
) -> Vec<RankedGame> {
    ranked_games
        .iter()
        .filter(|ranked_game| standing(votes, ranked_game.id, now).is_none())
        .filter_map(|ranked_game| {
            let opinion = *opinions.get(&ranked_game.id)?;
            if opinion >= 0.0 {
                return None;
            }
            let mut bet = ranked_game.clone();
            bet.relevance *= (votes_weight * opinion).exp();
            Some(bet)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::votes::{LATER_DAYS, Vote, VoteKind};

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const CAST: i64 = 1_794_404_475_152;

    fn ranked(id: u32) -> RankedGame {
        RankedGame {
            id: AppId(id),
            relevance: 1.0,
            affinity: 1.0,
        }
    }

    fn vote(kind: VoteKind) -> Vote {
        Vote {
            at: Some(Timestamp(CAST)),
            ..Vote::new(kind, "2026-11-11")
        }
    }

    fn bet_ids(now: i64) -> Vec<u32> {
        let games = [ranked(1), ranked(2), ranked(3)];
        let opinions = HashMap::from([(AppId(1), -1.0), (AppId(2), -1.0), (AppId(3), -1.0)]);
        let votes = Votes::from([
            (AppId(2), vote(VoteKind::Later)),
            (AppId(3), vote(VoteKind::NotForMe)),
        ]);

        bets(&games, &opinions, &votes, 1.0, Timestamp(now))
            .iter()
            .map(|game| game.id.0)
            .collect()
    }

    #[test]
    fn a_later_keeps_a_game_out_of_the_bets_until_it_lapses() {
        let lapse = CAST + LATER_DAYS * DAY_MS;

        assert_eq!(bet_ids(lapse - 1), [1]);
        assert_eq!(bet_ids(lapse), [1, 2]);
    }
}
