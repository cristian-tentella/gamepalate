use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use gamepalate_app::{AppId, GameDetails};

const MAX_PAGES: usize = 50;

pub enum Page {
    Loading,
    Ready(Arc<GameDetails>),
    Missing,
    Failed,
}

#[derive(Default)]
pub struct Pages {
    pages: HashMap<AppId, Page>,
}

impl Pages {
    pub fn get(&self, id: AppId) -> Option<&Page> {
        self.pages.get(&id)
    }

    pub fn details(&self, id: AppId) -> Option<&GameDetails> {
        match self.pages.get(&id) {
            Some(Page::Ready(details)) => Some(details),
            Some(Page::Loading | Page::Missing | Page::Failed) | None => None,
        }
    }

    pub fn want(&mut self, ids: &[AppId]) -> Vec<AppId> {
        let mut started = Vec::new();
        for &id in ids {
            if let Entry::Vacant(entry) = self.pages.entry(id) {
                entry.insert(Page::Loading);
                started.push(id);
            }
        }
        started
    }

    pub fn loaded(&mut self, id: AppId, result: Result<Option<Arc<GameDetails>>, String>) {
        if let Some(page) = self.pages.get_mut(&id) {
            *page = match result {
                Ok(Some(details)) => Page::Ready(details),
                Ok(None) => Page::Missing,
                Err(_) => Page::Failed,
            };
        }
    }

    pub fn retry(&mut self, id: AppId) {
        if matches!(self.pages.get(&id), Some(Page::Failed)) {
            self.pages.remove(&id);
        }
    }

    pub fn keep_only(&mut self, nearby: &[AppId]) {
        if self.pages.len() <= MAX_PAGES {
            return;
        }
        self.pages
            .retain(|id, page| matches!(page, Page::Loading) || nearby.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::ItemDetails;

    use super::*;

    fn details() -> Result<Option<Arc<GameDetails>>, String> {
        Ok(Some(Arc::new(GameDetails {
            store: ItemDetails {
                short_description: None,
                developers: Vec::new(),
                release_date: None,
                header_url: None,
                screenshot_urls: vec!["https://example.com/1.jpg".to_string()],
                player_categories: Vec::new(),
                controller: None,
                italian: None,
                steam_deck: None,
            },
            modes: Vec::new(),
        })))
    }

    #[test]
    fn asks_once_for_each_game() {
        let mut pages = Pages::default();

        assert_eq!(
            pages.want(&[AppId(1), AppId(2), AppId(1)]),
            [AppId(1), AppId(2)]
        );
        assert!(pages.want(&[AppId(1), AppId(2)]).is_empty());
        assert!(matches!(pages.get(AppId(1)), Some(Page::Loading)));
    }

    #[test]
    fn keeps_what_arrives() {
        let mut pages = Pages::default();
        pages.want(&[AppId(1), AppId(2), AppId(3)]);

        pages.loaded(AppId(1), details());
        pages.loaded(AppId(2), Ok(None));
        pages.loaded(AppId(3), Err("down".to_string()));

        assert_eq!(
            pages.details(AppId(1)).unwrap().store.screenshot_urls.len(),
            1
        );
        assert!(matches!(pages.get(AppId(2)), Some(Page::Missing)));
        assert!(matches!(pages.get(AppId(3)), Some(Page::Failed)));
        assert!(pages.want(&[AppId(2), AppId(3)]).is_empty());
    }

    #[test]
    fn asks_again_after_a_retry_only_if_it_failed() {
        let mut pages = Pages::default();
        pages.want(&[AppId(1), AppId(2)]);
        pages.loaded(AppId(1), Err("down".to_string()));
        pages.loaded(AppId(2), details());

        pages.retry(AppId(1));
        pages.retry(AppId(2));

        assert_eq!(pages.want(&[AppId(1), AppId(2)]), [AppId(1)]);
    }

    #[test]
    fn forgets_the_games_far_away_over_the_limit() {
        let mut pages = Pages::default();
        for id in 1..=MAX_PAGES as u32 + 1 {
            pages.want(&[AppId(id)]);
            pages.loaded(AppId(id), Ok(None));
        }
        pages.want(&[AppId(500)]);

        pages.keep_only(&[AppId(1)]);

        assert!(pages.get(AppId(1)).is_some());
        assert!(pages.get(AppId(500)).is_some());
        assert!(pages.get(AppId(2)).is_none());
    }

    #[test]
    fn ignores_details_for_a_forgotten_game() {
        let mut pages = Pages::default();

        pages.loaded(AppId(1), details());

        assert!(pages.get(AppId(1)).is_none());
    }
}
