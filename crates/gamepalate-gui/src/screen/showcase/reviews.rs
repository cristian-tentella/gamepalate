use std::{collections::HashMap, sync::Arc};

use gamepalate_app::{
    AppId,
    reviews::{Review, ReviewPage, ReviewQuery},
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
    cursor: Option<String>,
}

/// One page asked to Steam: `cursor` is `None` for the first one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: AppId,
    pub query: ReviewQuery,
    pub cursor: Option<String>,
}

#[derive(Default)]
pub struct ReviewLists {
    lists: HashMap<(AppId, ReviewQuery), ReviewList>,
}

impl ReviewLists {
    pub fn get(&self, id: AppId, query: ReviewQuery) -> Option<&ReviewList> {
        self.lists.get(&(id, query))
    }

    pub fn start(&mut self, id: AppId, query: ReviewQuery) -> Option<Request> {
        if self.lists.contains_key(&(id, query)) {
            return None;
        }
        self.more(id, query)
    }

    pub fn more(&mut self, id: AppId, query: ReviewQuery) -> Option<Request> {
        let list = self.lists.entry((id, query)).or_insert(ReviewList {
            reviews: Vec::new(),
            fetch: Fetch::Idle,
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

    pub fn retry(&mut self, id: AppId, query: ReviewQuery) -> Option<Request> {
        let list = self.lists.get_mut(&(id, query))?;
        if list.fetch == Fetch::Failed {
            list.fetch = Fetch::Idle;
        }
        self.more(id, query)
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
    use gamepalate_app::{Timestamp, reviews::Order};

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
            .get(id, ReviewQuery::default())
            .map(|list| {
                list.reviews
                    .iter()
                    .map(|review| review.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn asks_the_first_page_once() {
        let mut lists = ReviewLists::default();
        let query = ReviewQuery::default();

        let first = lists.start(AppId(1), query).unwrap();

        assert_eq!(first.cursor, None);
        assert!(lists.start(AppId(1), query).is_none());
        assert!(lists.more(AppId(1), query).is_none());
        assert_eq!(
            lists.get(AppId(1), query).map(|list| list.fetch),
            Some(Fetch::Loading)
        );
    }

    #[test]
    fn goes_on_from_the_cursor_until_the_last_page() {
        let mut lists = ReviewLists::default();
        let query = ReviewQuery::default();
        let first = lists.start(AppId(1), query).unwrap();

        lists.loaded(first, page(&["a", "b"], Some("next")));
        let second = lists.more(AppId(1), query).unwrap();
        assert_eq!(second.cursor.as_deref(), Some("next"));
        lists.loaded(second, page(&["c"], None));

        assert_eq!(ids(&lists, AppId(1)), ["a", "b", "c"]);
        assert_eq!(
            lists.get(AppId(1), query).map(|list| list.fetch),
            Some(Fetch::Finished)
        );
        assert!(lists.more(AppId(1), query).is_none());
    }

    #[test]
    fn an_error_keeps_what_arrived_and_retry_asks_the_same_page() {
        let mut lists = ReviewLists::default();
        let query = ReviewQuery::default();
        let first = lists.start(AppId(1), query).unwrap();
        lists.loaded(first, page(&["a"], Some("next")));
        let second = lists.more(AppId(1), query).unwrap();

        lists.loaded(second, Err("down".to_string()));

        assert_eq!(
            lists.get(AppId(1), query).map(|list| list.fetch),
            Some(Fetch::Failed)
        );
        assert!(lists.more(AppId(1), query).is_none());
        let again = lists.retry(AppId(1), query).unwrap();
        assert_eq!(again.cursor.as_deref(), Some("next"));
        assert_eq!(ids(&lists, AppId(1)), ["a"]);
    }

    #[test]
    fn a_late_answer_for_another_page_or_query_is_dropped() {
        let mut lists = ReviewLists::default();
        let query = ReviewQuery::default();
        let first = lists.start(AppId(1), query).unwrap();
        lists.loaded(first.clone(), page(&["a"], Some("next")));

        lists.loaded(first, page(&["a"], Some("next")));
        lists.loaded(
            Request {
                id: AppId(1),
                query: ReviewQuery {
                    order: Order::Recent,
                    ..query
                },
                cursor: None,
            },
            page(&["x"], None),
        );

        assert_eq!(ids(&lists, AppId(1)), ["a"]);
        assert!(
            lists
                .get(
                    AppId(1),
                    ReviewQuery {
                        order: Order::Recent,
                        ..query
                    }
                )
                .is_none()
        );
    }

    #[test]
    fn keeps_the_answers_of_the_nearby_games_only() {
        let mut lists = ReviewLists::default();
        let query = ReviewQuery::default();
        let near = lists.start(AppId(1), query).unwrap();
        let far = lists.start(AppId(2), query).unwrap();

        lists.keep_only(&[AppId(1)]);
        lists.loaded(near, page(&["a"], None));
        lists.loaded(far, page(&["b"], None));

        assert_eq!(ids(&lists, AppId(1)), ["a"]);
        assert!(lists.get(AppId(2), query).is_none());
    }
}
