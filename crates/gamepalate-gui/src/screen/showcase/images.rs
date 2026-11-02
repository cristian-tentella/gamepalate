use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
};

use iced::widget::image::Allocation;

const MAX_LOADS: usize = 16;
const MAX_PICTURES: usize = 100;

pub enum Picture<T = Allocation> {
    Loading,
    Ready(T),
    Failed,
}

pub struct Images<T = Allocation, K = String> {
    pictures: HashMap<K, Picture<T>>,
    waiting: VecDeque<K>,
}

impl<T, K> Default for Images<T, K> {
    fn default() -> Self {
        Images {
            pictures: HashMap::new(),
            waiting: VecDeque::new(),
        }
    }
}

impl<T, K: Hash + Eq + Clone> Images<T, K> {
    pub fn get<Q: Hash + Eq + ?Sized>(&self, url: &Q) -> Option<&Picture<T>>
    where
        K: std::borrow::Borrow<Q>,
    {
        self.pictures.get(url)
    }

    pub fn want(&mut self, urls: Vec<K>) -> Vec<K> {
        let mut seen = HashSet::new();
        self.waiting = urls
            .into_iter()
            .filter(|url| !self.pictures.contains_key(url) && seen.insert(url.clone()))
            .collect();

        let mut started = Vec::new();
        while self.loads() < MAX_LOADS {
            let Some(url) = self.waiting.pop_front() else {
                break;
            };
            self.pictures.insert(url.clone(), Picture::Loading);
            started.push(url);
        }
        started
    }

    pub fn loaded(&mut self, url: K, image: Option<T>) {
        if let Some(picture) = self.pictures.get_mut(&url) {
            *picture = match image {
                Some(image) => Picture::Ready(image),
                None => Picture::Failed,
            };
        }
    }

    pub fn keep_only(&mut self, nearby: &[K]) {
        if self.pictures.len() <= MAX_PICTURES {
            return;
        }
        self.pictures
            .retain(|url, picture| matches!(picture, Picture::Loading) || nearby.contains(url));
    }

    fn loads(&self) -> usize {
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

    fn handle() -> Option<u8> {
        Some(0)
    }

    #[test]
    fn starts_at_most_sixteen_loads_at_once() {
        let mut images = Images::<u8>::default();

        assert_eq!(images.want(urls(1..=18)), urls(1..=16));
        assert!(images.want(urls(1..=18)).is_empty());

        images.loaded(url(1), handle());
        assert_eq!(images.want(urls(1..=18)), vec![url(17)]);

        images.loaded(url(2), None);
        assert_eq!(images.want(urls(1..=18)), vec![url(18)]);
    }

    #[test]
    fn asks_for_the_same_address_once() {
        let mut images = Images::<u8>::default();

        assert_eq!(images.want(vec![url(1), url(1)]), vec![url(1)]);
        images.loaded(url(1), None);

        assert!(images.want(vec![url(1)]).is_empty());
        assert!(matches!(images.get(&url(1)), Some(Picture::Failed)));
    }

    #[test]
    fn puts_the_current_game_first() {
        let mut images = Images::<u8>::default();
        images.want(urls(1..=6));

        images.loaded(url(1), handle());
        assert_eq!(
            images.want([vec![url(9)], urls(2..=6)].concat()),
            vec![url(9)]
        );
    }

    #[test]
    fn forgets_the_games_far_away_over_the_limit() {
        let mut images = Images::<u8>::default();
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
        let mut images = Images::<u8>::default();
        images.want(vec![url(1)]);
        images.loaded(url(1), handle());

        images.keep_only(&[]);

        assert!(images.get(&url(1)).is_some());
    }

    #[test]
    fn ignores_an_image_that_was_forgotten() {
        let mut images = Images::<u8>::default();

        images.loaded(url(1), handle());

        assert!(images.get(&url(1)).is_none());
    }
}
