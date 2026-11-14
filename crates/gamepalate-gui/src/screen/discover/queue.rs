use gamepalate_app::{Continuation, RecommendedGame};

pub struct Queue {
    games: Vec<RecommendedGame>,
    index: usize,
    next: Next,
    // Where the next page goes on from
    continuation: Continuation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Next {
    Idle,
    Loading,
    Waiting,
    Exhausted,
}

impl Queue {
    pub fn new(games: Vec<RecommendedGame>, continuation: Continuation) -> Queue {
        Queue {
            games,
            index: 0,
            next: Next::Idle,
            continuation,
        }
    }

    pub fn current(&self) -> Option<&RecommendedGame> {
        self.games.get(self.index)
    }

    pub fn upcoming(&self, ahead: usize) -> &[RecommendedGame] {
        self.around(0, ahead)
    }

    pub fn around(&self, behind: usize, ahead: usize) -> &[RecommendedGame] {
        let start = self.index.saturating_sub(behind).min(self.games.len());
        let end = (self.index + ahead + 1).min(self.games.len());
        &self.games[start..end]
    }

    pub fn is_waiting(&self) -> bool {
        self.next == Next::Waiting
    }

    pub fn advance(&mut self) -> bool {
        if self.index + 1 < self.games.len() {
            self.index += 1;
            return self.wants_next();
        }

        match self.next {
            Next::Idle => {
                self.next = Next::Waiting;
                true
            }
            Next::Loading => {
                self.next = Next::Waiting;
                false
            }
            Next::Waiting | Next::Exhausted => false,
        }
    }

    pub fn back(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    pub fn wants_next(&mut self) -> bool {
        let at_last = self.index + 1 >= self.games.len();
        if at_last && !self.games.is_empty() && self.next == Next::Idle {
            self.next = Next::Loading;
            return true;
        }
        false
    }

    pub fn continuation(&self) -> &Continuation {
        &self.continuation
    }

    pub fn receive(&mut self, games: Vec<RecommendedGame>, continuation: Continuation) {
        if games.is_empty() {
            self.next = Next::Exhausted;
            return;
        }

        let waiting = self.next == Next::Waiting;
        self.games.extend(games);
        self.continuation = continuation;
        self.next = Next::Idle;
        if waiting {
            self.index += 1;
        }
    }

    pub fn failed(&mut self) {
        self.next = Next::Idle;
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::{AppId, Price, Reviews};

    use super::*;

    #[test]
    fn asks_for_the_next_batch_on_the_last_game() {
        let mut queue = Queue::new(games(1..=2), Continuation::default());

        assert!(queue.advance());
        assert!(!queue.advance());
        assert!(queue.is_waiting());
    }

    #[test]
    fn moves_to_the_new_batch_when_it_was_waiting() {
        let mut queue = Queue::new(games(1..=2), Continuation::default());
        queue.advance();
        queue.advance();

        queue.receive(games(3..=5), after(3..=5));

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(3)));
    }

    #[test]
    fn stays_put_when_the_preloaded_batch_arrives_early() {
        let mut queue = Queue::new(games(1..=2), Continuation::default());
        queue.advance();

        queue.receive(games(3..=4), after(3..=4));

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(2)));
    }

    #[test]
    fn goes_back_across_batches() {
        let mut queue = Queue::new(games(1..=1), Continuation::default());
        queue.advance();
        queue.receive(games(2..=3), after(2..=3));

        queue.back();

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(1)));
    }

    #[test]
    fn stops_when_there_are_no_more_games() {
        let mut queue = Queue::new(games(1..=1), Continuation::default());
        queue.advance();

        queue.receive(Vec::new(), Continuation::default());

        assert!(!queue.advance());
        assert!(!queue.is_waiting());
    }

    #[test]
    fn lists_the_current_game_and_the_next_ones() {
        let mut queue = Queue::new(games(1..=4), Continuation::default());
        queue.advance();

        assert_eq!(ids(queue.upcoming(5)), vec![2, 3, 4]);
        assert_eq!(ids(queue.around(1, 1)), vec![1, 2, 3]);
        assert_eq!(ids(queue.around(9, 0)), vec![1, 2]);
    }

    #[test]
    fn keeps_the_continuation_of_the_last_batch() {
        let mut queue = Queue::new(games(1..=2), after(1..=2));
        queue.advance();

        queue.receive(games(3..=4), after(3..=4));

        assert_eq!(queue.continuation(), &after(3..=4));
    }

    #[test]
    fn keeps_the_continuation_when_no_games_are_left() {
        let mut queue = Queue::new(games(1..=1), after(1..=1));
        queue.advance();

        queue.receive(Vec::new(), Continuation::default());

        assert_eq!(queue.continuation(), &after(1..=1));
    }

    fn ids(games: &[RecommendedGame]) -> Vec<u32> {
        games.iter().map(|game| game.id.0).collect()
    }

    // As if the core had shown the games up to the last of these
    fn after(ids: std::ops::RangeInclusive<u32>) -> Continuation {
        Continuation {
            shown: (1..=*ids.end()).map(AppId).collect(),
            turn: *ids.end() as usize,
            bet_turn: 0,
            places: *ids.end() as usize,
        }
    }

    fn games(ids: std::ops::RangeInclusive<u32>) -> Vec<RecommendedGame> {
        ids.map(|id| RecommendedGame {
            position: id as usize,
            id: AppId(id),
            name: format!("Game {id}"),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            store_url: String::new(),
            from_family: None,
            affinity: 1.0,
            tags: Vec::new(),
            similar_to: None,
            similar_to_id: None,
            similar_to_known_from: None,
            similar_to_multiplier: None,
            quality: 1.0,
            reviews: Reviews {
                total: 100,
                positive: 90,
            },
            in_wishlist: false,
            relevance: 1.0,
            bet: false,
        })
        .collect()
    }
}
