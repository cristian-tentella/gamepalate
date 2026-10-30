use std::collections::{HashMap, HashSet, VecDeque};

use iced::widget::image::Handle;

const MAX_DOWNLOADS: usize = 4;
const MAX_PICTURES: usize = 100;

pub enum Picture {
    Loading,
    Ready(Handle),
    Failed,
}

#[derive(Default)]
pub struct Images {
    pictures: HashMap<String, Picture>,
    waiting: VecDeque<String>,
}

impl Images {
    pub fn get(&self, url: &str) -> Option<&Picture> {
        self.pictures.get(url)
    }

    pub fn want(&mut self, urls: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        self.waiting = urls
            .into_iter()
            .filter(|url| !self.pictures.contains_key(url) && seen.insert(url.clone()))
            .collect();

        let mut started = Vec::new();
        while self.downloads() < MAX_DOWNLOADS {
            let Some(url) = self.waiting.pop_front() else {
                break;
            };
            self.pictures.insert(url.clone(), Picture::Loading);
            started.push(url);
        }
        started
    }

    pub fn loaded(&mut self, url: String, handle: Option<Handle>) {
        if let Some(picture) = self.pictures.get_mut(&url) {
            *picture = match handle {
                Some(handle) => Picture::Ready(handle),
                None => Picture::Failed,
            };
        }
    }

    pub fn keep_only(&mut self, nearby: &[String]) {
        if self.pictures.len() <= MAX_PICTURES {
            return;
        }
        self.pictures
            .retain(|url, picture| matches!(picture, Picture::Loading) || nearby.contains(url));
    }

    fn downloads(&self) -> usize {
        self.pictures
            .values()
            .filter(|picture| matches!(picture, Picture::Loading))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(range: std::ops::RangeInclusive<u32>) -> Vec<String> {
        range
            .map(|id| format!("https://example.com/{id}.jpg"))
            .collect()
    }

    fn url(id: u32) -> String {
        format!("https://example.com/{id}.jpg")
    }

    fn handle() -> Option<Handle> {
        Some(Handle::from_bytes(vec![0]))
    }

    #[test]
    fn starts_at_most_four_downloads_at_once() {
        let mut images = Images::default();

        assert_eq!(images.want(urls(1..=6)), urls(1..=4));
        assert!(images.want(urls(1..=6)).is_empty());

        images.loaded(url(1), handle());
        assert_eq!(images.want(urls(1..=6)), vec![url(5)]);

        images.loaded(url(2), None);
        assert_eq!(images.want(urls(1..=6)), vec![url(6)]);
    }

    #[test]
    fn asks_for_the_same_address_once() {
        let mut images = Images::default();

        assert_eq!(images.want(vec![url(1), url(1)]), vec![url(1)]);
        images.loaded(url(1), None);

        assert!(images.want(vec![url(1)]).is_empty());
        assert!(matches!(images.get(&url(1)), Some(Picture::Failed)));
    }

    #[test]
    fn puts_the_current_game_first() {
        let mut images = Images::default();
        images.want(urls(1..=6));

        images.loaded(url(1), handle());
        assert_eq!(
            images.want([vec![url(9)], urls(2..=6)].concat()),
            vec![url(9)]
        );
    }

    #[test]
    fn forgets_the_games_far_away_over_the_limit() {
        let mut images = Images::default();
        for id in 1..=MAX_PICTURES as u32 + 1 {
            images.want(vec![url(id)]);
            images.loaded(url(id), handle());
        }
        images.want(vec![url(500)]);

        images.keep_only(&[url(1)]);

        assert!(images.get(&url(1)).is_some());
        assert!(images.get(&url(500)).is_some());
        assert!(images.get(&url(2)).is_none());
    }

    #[test]
    fn keeps_everything_under_the_limit() {
        let mut images = Images::default();
        images.want(vec![url(1)]);
        images.loaded(url(1), handle());

        images.keep_only(&[]);

        assert!(images.get(&url(1)).is_some());
    }

    #[test]
    fn ignores_an_image_that_was_forgotten() {
        let mut images = Images::default();

        images.loaded(url(1), handle());

        assert!(images.get(&url(1)).is_none());
    }
}
