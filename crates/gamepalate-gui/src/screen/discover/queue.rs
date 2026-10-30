use gamepalate_app::{AppId, RecommendedGame};

pub struct Queue {
    games: Vec<RecommendedGame>,
    batch_starts: Vec<usize>,
    index: usize,
    next: Next,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Next {
    Idle,
    Loading,
    Waiting,
    Exhausted,
}

impl Queue {
    pub fn new(games: Vec<RecommendedGame>) -> Queue {
        Queue {
            games,
            batch_starts: vec![0],
            index: 0,
            next: Next::Idle,
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

    pub fn position(&self) -> (usize, usize) {
        let start = self
            .batch_starts
            .iter()
            .rev()
            .find(|&&start| start <= self.index)
            .copied()
            .unwrap_or(0);
        let end = self
            .batch_starts
            .iter()
            .find(|&&start| start > self.index)
            .copied()
            .unwrap_or(self.games.len());

        (self.index - start + 1, end - start)
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

    pub fn seen(&self) -> Vec<AppId> {
        self.games.iter().map(|game| game.id).collect()
    }

    pub fn receive(&mut self, games: Vec<RecommendedGame>) {
        if games.is_empty() {
            self.next = Next::Exhausted;
            return;
        }

        let waiting = self.next == Next::Waiting;
        self.batch_starts.push(self.games.len());
        self.games.extend(games);
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
    use gamepalate_app::{Price, Reviews};

    use super::*;

    #[test]
    fn counts_the_position_within_the_batch() {
        let mut queue = Queue::new(games(1..=3));
        queue.advance();

        assert_eq!(queue.position(), (2, 3));
    }

    #[test]
    fn asks_for_the_next_batch_on_the_last_game() {
        let mut queue = Queue::new(games(1..=2));

        assert!(queue.advance());
        assert!(!queue.advance());
        assert!(queue.is_waiting());
    }

    #[test]
    fn moves_to_the_new_batch_when_it_was_waiting() {
        let mut queue = Queue::new(games(1..=2));
        queue.advance();
        queue.advance();

        queue.receive(games(3..=5));

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(3)));
        assert_eq!(queue.position(), (1, 3));
    }

    #[test]
    fn stays_put_when_the_preloaded_batch_arrives_early() {
        let mut queue = Queue::new(games(1..=2));
        queue.advance();

        queue.receive(games(3..=4));

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(2)));
        assert_eq!(queue.seen().len(), 4);
    }

    #[test]
    fn goes_back_across_batches() {
        let mut queue = Queue::new(games(1..=1));
        queue.advance();
        queue.receive(games(2..=3));

        queue.back();

        assert_eq!(queue.current().map(|game| game.id), Some(AppId(1)));
        assert_eq!(queue.position(), (1, 1));
    }

    #[test]
    fn stops_when_there_are_no_more_games() {
        let mut queue = Queue::new(games(1..=1));
        queue.advance();

        queue.receive(Vec::new());

        assert!(!queue.advance());
        assert!(!queue.is_waiting());
    }

    #[test]
    fn lists_the_current_game_and_the_next_ones() {
        let mut queue = Queue::new(games(1..=4));
        queue.advance();

        assert_eq!(ids(queue.upcoming(5)), vec![2, 3, 4]);
        assert_eq!(ids(queue.around(1, 1)), vec![1, 2, 3]);
        assert_eq!(ids(queue.around(9, 0)), vec![1, 2]);
    }

    fn ids(games: &[RecommendedGame]) -> Vec<u32> {
        games.iter().map(|game| game.id.0).collect()
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
            top_tags: Vec::new(),
            similar_to: None,
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
