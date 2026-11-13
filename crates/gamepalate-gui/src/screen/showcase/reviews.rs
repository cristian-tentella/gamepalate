use std::{collections::HashMap, sync::Arc};

use gamepalate_app::{
    AppId,
    reviews::{Order, Review, ReviewPage, ReviewQuery},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    Idle,
    Loading,
    Finished,
    Failed,
}

pub struct ReviewList {
    pub reviews: Vec<Review>,
    pub fetch: Fetch,
    pub matching: Option<u32>,
    cursor: Option<String>,
}

impl ReviewList {
    pub fn found_none(&self) -> bool {
        self.fetch == Fetch::Finished && self.reviews.is_empty()
    }
}

/// One page asked to Steam: `cursor` is `None` for the first one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: AppId,
    pub query: ReviewQuery,
    pub cursor: Option<String>,
}

/// The lists already asked, for every nearby game and query, and the filters chosen in the sheet.
#[derive(Default)]
pub struct ReviewLists {
    lists: HashMap<(AppId, ReviewQuery), ReviewList>,
    query: ReviewQuery,
}

impl ReviewLists {
    pub fn query(&self) -> ReviewQuery {
        self.query
    }

    pub fn choose(&mut self, query: ReviewQuery) -> bool {
        let changed = query != self.query;
        self.query = query;
        changed
    }

    /// The query the sheet shows: Most helpful only looks at the past year, so with nothing there
    /// it shows the most recent ones.
    pub fn shown(&self, id: AppId) -> ReviewQuery {
        let helpful_is_empty = self.query.order == Order::Helpful
            && self
                .lists
                .get(&(id, self.query))
                .is_some_and(ReviewList::found_none);
        if helpful_is_empty {
            ReviewQuery {
                order: Order::Recent,
                ..self.query
            }
        } else {
            self.query
        }
    }

    pub fn fell_back(&self, id: AppId) -> bool {
        self.shown(id) != self.query
    }

    pub fn get(&self, id: AppId) -> Option<&ReviewList> {
        self.lists.get(&(id, self.shown(id)))
    }

    pub fn start(&mut self, id: AppId) -> Option<Request> {
        if self.lists.contains_key(&(id, self.shown(id))) {
            return None;
        }
        self.more(id)
    }

    pub fn more(&mut self, id: AppId) -> Option<Request> {
        let query = self.shown(id);
        let list = self.lists.entry((id, query)).or_insert(ReviewList {
            reviews: Vec::new(),
            fetch: Fetch::Idle,
            matching: None,
            cursor: None,
        });
        if list.fetch != Fetch::Idle {
            return None;
        }
        list.fetch = Fetch::Loading;

        Some(Request {
            id,
            query,
            cursor: list.cursor.clone(),
        })
    }

    pub fn retry(&mut self, id: AppId) -> Option<Request> {
        let list = self.lists.get_mut(&(id, self.shown(id)))?;
        if list.fetch == Fetch::Failed {
            list.fetch = Fetch::Idle;
        }
        self.more(id)
    }

    pub fn loaded(&mut self, request: Request, result: Result<Arc<ReviewPage>, String>) {
        let Some(list) = self.lists.get_mut(&(request.id, request.query)) else {
            return;
        };
        if list.fetch != Fetch::Loading || list.cursor != request.cursor {
            return;
        }
        match result {
            Ok(page) => {
                if request.cursor.is_none() {
                    list.matching = page.total_matching;
                }
                list.reviews.extend(page.reviews.iter().cloned());
                list.fetch = if page.next.is_some() {
                    Fetch::Idle
                } else {
                    Fetch::Finished
                };
                list.cursor.clone_from(&page.next);
            }
            Err(_) => list.fetch = Fetch::Failed,
        }
    }

    pub fn keep_only(&mut self, nearby: &[AppId]) {
        self.lists.retain(|(id, _), _| nearby.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::{
        Timestamp,
        reviews::{Languages, Verdict},
    };

    use super::*;

    fn review(id: &str) -> Review {
        Review {
            id: id.to_string(),
            recommended: true,
            text: Vec::new(),
            hours_at_review: 1.0,
            hours_now: 2.0,
            written: Timestamp(0),
            helpful: 0,
            language: "english".to_string(),
        }
    }

    fn page(ids: &[&str], next: Option<&str>) -> Result<Arc<ReviewPage>, String> {
        Ok(Arc::new(ReviewPage {
            reviews: ids.iter().map(|id| review(id)).collect(),
            next: next.map(str::to_string),
            total_matching: Some(25),
        }))
    }

    fn ids(lists: &ReviewLists, id: AppId) -> Vec<String> {
        lists
            .get(id)
            .map(|list| {
                list.reviews
                    .iter()
                    .map(|review| review.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn fetch(lists: &ReviewLists, id: AppId) -> Option<Fetch> {
        lists.get(id).map(|list| list.fetch)
    }

    fn recent() -> ReviewQuery {
        ReviewQuery {
            order: Order::Recent,
            ..ReviewQuery::default()
        }
    }

    fn negative() -> ReviewQuery {
        ReviewQuery {
            verdict: Verdict::Negative,
            ..ReviewQuery::default()
        }
    }

    #[test]
    fn asks_the_first_page_once() {
        let mut lists = ReviewLists::default();

        let first = lists.start(AppId(1)).unwrap();

        assert_eq!(first.query, ReviewQuery::default());
        assert_eq!(first.cursor, None);
        assert!(lists.start(AppId(1)).is_none());
        assert!(lists.more(AppId(1)).is_none());
        assert_eq!(fetch(&lists, AppId(1)), Some(Fetch::Loading));
    }

    #[test]
    fn goes_on_from_the_cursor_until_the_last_page() {
        let mut lists = ReviewLists::default();
        let first = lists.start(AppId(1)).unwrap();

        lists.loaded(first, page(&["a", "b"], Some("next")));
        let second = lists.more(AppId(1)).unwrap();
        assert_eq!(second.cursor.as_deref(), Some("next"));
        lists.loaded(second, page(&["c"], None));

        assert_eq!(ids(&lists, AppId(1)), ["a", "b", "c"]);
        assert_eq!(fetch(&lists, AppId(1)), Some(Fetch::Finished));
        assert!(lists.more(AppId(1)).is_none());
    }

    #[test]
    fn keeps_the_number_of_matching_reviews_from_the_first_page() {
        let mut lists = ReviewLists::default();
        let first = lists.start(AppId(1)).unwrap();
        lists.loaded(first, page(&["a"], Some("next")));
        let second = lists.more(AppId(1)).unwrap();

        lists.loaded(
            second,
            Ok(Arc::new(ReviewPage {
                reviews: vec![review("b")],
                next: None,
                total_matching: None,
            })),
        );

        assert_eq!(lists.get(AppId(1)).and_then(|list| list.matching), Some(25));
    }

    #[test]
    fn an_error_keeps_what_arrived_and_retry_asks_the_same_page() {
        let mut lists = ReviewLists::default();
        let first = lists.start(AppId(1)).unwrap();
        lists.loaded(first, page(&["a"], Some("next")));
        let second = lists.more(AppId(1)).unwrap();

        lists.loaded(second, Err("down".to_string()));

        assert_eq!(fetch(&lists, AppId(1)), Some(Fetch::Failed));
        assert!(lists.more(AppId(1)).is_none());
        let again = lists.retry(AppId(1)).unwrap();
        assert_eq!(again.cursor.as_deref(), Some("next"));
        assert_eq!(ids(&lists, AppId(1)), ["a"]);
    }

    #[test]
    fn a_late_answer_for_another_page_or_query_is_dropped() {
        let mut lists = ReviewLists::default();
        let first = lists.start(AppId(1)).unwrap();
        lists.loaded(first.clone(), page(&["a"], Some("next")));

        lists.loaded(first, page(&["a"], Some("next")));
        lists.loaded(
            Request {
                id: AppId(1),
                query: recent(),
                cursor: None,
            },
            page(&["x"], None),
        );

        assert_eq!(ids(&lists, AppId(1)), ["a"]);
        lists.choose(recent());
        assert!(lists.get(AppId(1)).is_none());
    }

    #[test]
    fn keeps_the_answers_of_the_nearby_games_only() {
        let mut lists = ReviewLists::default();
        let near = lists.start(AppId(1)).unwrap();
        let far = lists.start(AppId(2)).unwrap();

        lists.keep_only(&[AppId(1)]);
        lists.loaded(near, page(&["a"], None));
        lists.loaded(far, page(&["b"], None));

        assert_eq!(ids(&lists, AppId(1)), ["a"]);
        assert!(lists.get(AppId(2)).is_none());
    }

    #[test]
    fn every_choice_asks_its_own_first_page() {
        let choices = [
            ReviewQuery {
                order: Order::Funny,
                ..ReviewQuery::default()
            },
            negative(),
            ReviewQuery {
                languages: Languages::All,
                ..ReviewQuery::default()
            },
            ReviewQuery {
                min_hours: Some(10),
                ..ReviewQuery::default()
            },
        ];
        for query in choices {
            let mut lists = ReviewLists::default();
            let first = lists.start(AppId(1)).unwrap();
            lists.loaded(first, page(&["a"], Some("next")));

            assert!(lists.choose(query));
            let request = lists.start(AppId(1)).unwrap();

            assert_eq!(request.query, query);
            assert_eq!(request.cursor, None);
            assert!(ids(&lists, AppId(1)).is_empty());
        }
    }

    #[test]
    fn choosing_the_same_filters_changes_nothing() {
        let mut lists = ReviewLists::default();

        assert!(!lists.choose(ReviewQuery::default()));
    }

    #[test]
    fn going_back_to_filters_already_seen_downloads_nothing() {
        let mut lists = ReviewLists::default();
        let first = lists.start(AppId(1)).unwrap();
        lists.loaded(first, page(&["a"], Some("next")));
        lists.choose(recent());
        let other = lists.start(AppId(1)).unwrap();
        lists.loaded(other, page(&["x"], None));

        lists.choose(ReviewQuery::default());

        assert!(lists.start(AppId(1)).is_none());
        assert_eq!(ids(&lists, AppId(1)), ["a"]);
    }

    #[test]
    fn the_filters_stay_from_one_game_to_the_next() {
        let mut lists = ReviewLists::default();
        lists.choose(negative());
        lists.start(AppId(1)).unwrap();

        lists.keep_only(&[AppId(2)]);
        let next = lists.start(AppId(2)).unwrap();

        assert_eq!(lists.query(), negative());
        assert_eq!(next.query, negative());
    }

    #[test]
    fn falls_back_to_the_most_recent_when_none_is_helpful() {
        let mut lists = ReviewLists::default();
        lists.choose(negative());
        let helpful = lists.start(AppId(1)).unwrap();
        assert!(!lists.fell_back(AppId(1)));

        lists.loaded(helpful, page(&[], None));
        let recent = lists.start(AppId(1)).unwrap();

        assert!(lists.fell_back(AppId(1)));
        assert_eq!(
            recent.query,
            ReviewQuery {
                order: Order::Recent,
                ..negative()
            }
        );
        assert_eq!(lists.query(), negative());
        lists.loaded(recent, page(&["old"], None));
        assert_eq!(ids(&lists, AppId(1)), ["old"]);
    }

    #[test]
    fn only_an_empty_most_helpful_falls_back() {
        let mut lists = ReviewLists::default();
        let helpful = lists.start(AppId(1)).unwrap();
        lists.loaded(helpful, page(&["a"], None));
        assert!(!lists.fell_back(AppId(1)));

        lists.choose(ReviewQuery {
            order: Order::Funny,
            ..ReviewQuery::default()
        });
        let first = lists.start(AppId(1)).unwrap();
        lists.loaded(first, page(&[], None));

        assert!(!lists.fell_back(AppId(1)));
        assert!(lists.start(AppId(1)).is_none());
        assert!(lists.get(AppId(1)).is_some_and(ReviewList::found_none));
    }

    #[test]
    fn a_failed_most_helpful_does_not_fall_back() {
        let mut lists = ReviewLists::default();
        let helpful = lists.start(AppId(1)).unwrap();

        lists.loaded(helpful, Err("down".to_string()));

        assert!(!lists.fell_back(AppId(1)));
        assert!(lists.start(AppId(1)).is_none());
    }
}
