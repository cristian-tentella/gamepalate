use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use gamepalate_app::{AppId, Block};

const MAX_DESCRIPTIONS: usize = 50;

pub enum Description {
    Loading,
    Ready(Arc<Vec<Block>>),
    Missing,
    Failed,
}

#[derive(Default)]
pub struct Descriptions {
    descriptions: HashMap<AppId, Description>,
}

impl Descriptions {
    pub fn get(&self, id: AppId) -> Option<&Description> {
        self.descriptions.get(&id)
    }

    pub fn image_urls(&self, id: AppId) -> Vec<String> {
        match self.descriptions.get(&id) {
            Some(Description::Ready(blocks)) => blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Image { url, .. } => Some(url.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn want(&mut self, ids: &[AppId]) -> Vec<AppId> {
        let mut started = Vec::new();
        for &id in ids {
            if let Entry::Vacant(entry) = self.descriptions.entry(id) {
                entry.insert(Description::Loading);
                started.push(id);
            }
        }
        started
    }

    pub fn loaded(&mut self, id: AppId, result: Result<Option<Arc<Vec<Block>>>, String>) {
        if let Some(description) = self.descriptions.get_mut(&id) {
            *description = match result {
                Ok(Some(blocks)) if !blocks.is_empty() => Description::Ready(blocks),
                Ok(_) => Description::Missing,
                Err(_) => Description::Failed,
            };
        }
    }

    pub fn keep_only(&mut self, nearby: &[AppId]) {
        if self.descriptions.len() <= MAX_DESCRIPTIONS {
            return;
        }
        self.descriptions.retain(|id, description| {
            matches!(description, Description::Loading) || nearby.contains(id)
        });
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_app::Span;

    use super::*;

    fn blocks() -> Result<Option<Arc<Vec<Block>>>, String> {
        Ok(Some(Arc::new(vec![
            Block::Paragraph(vec![Span {
                text: "A farm game.".to_string(),
                ..Span::default()
            }]),
            Block::Image {
                url: "https://example.com/a.avif".to_string(),
                width: 600,
                height: 105,
            },
            Block::Video {
                url: "https://example.com/v.mp4".to_string(),
            },
            Block::Image {
                url: "https://example.com/b.webp".to_string(),
                width: 616,
                height: 27,
            },
        ])))
    }

    #[test]
    fn asks_once_for_each_game() {
        let mut descriptions = Descriptions::default();

        assert_eq!(
            descriptions.want(&[AppId(1), AppId(2), AppId(1)]),
            [AppId(1), AppId(2)]
        );
        assert!(descriptions.want(&[AppId(1), AppId(2)]).is_empty());
        assert!(matches!(
            descriptions.get(AppId(1)),
            Some(Description::Loading)
        ));
    }

    #[test]
    fn keeps_what_arrives_and_an_empty_one_is_missing() {
        let mut descriptions = Descriptions::default();
        descriptions.want(&[AppId(1), AppId(2), AppId(3), AppId(4)]);

        descriptions.loaded(AppId(1), blocks());
        descriptions.loaded(AppId(2), Ok(None));
        descriptions.loaded(AppId(3), Ok(Some(Arc::new(Vec::new()))));
        descriptions.loaded(AppId(4), Err("down".to_string()));

        assert!(matches!(
            descriptions.get(AppId(1)),
            Some(Description::Ready(_))
        ));
        assert!(matches!(
            descriptions.get(AppId(2)),
            Some(Description::Missing)
        ));
        assert!(matches!(
            descriptions.get(AppId(3)),
            Some(Description::Missing)
        ));
        assert!(matches!(
            descriptions.get(AppId(4)),
            Some(Description::Failed)
        ));
    }

    #[test]
    fn gives_the_images_in_their_order() {
        let mut descriptions = Descriptions::default();
        descriptions.want(&[AppId(1)]);
        descriptions.loaded(AppId(1), blocks());

        assert_eq!(
            descriptions.image_urls(AppId(1)),
            ["https://example.com/a.avif", "https://example.com/b.webp"]
        );
        assert!(descriptions.image_urls(AppId(2)).is_empty());
    }

    #[test]
    fn forgets_the_games_far_away_over_the_limit() {
        let mut descriptions = Descriptions::default();
        for id in 1..=MAX_DESCRIPTIONS as u32 + 1 {
            descriptions.want(&[AppId(id)]);
            descriptions.loaded(AppId(id), Ok(None));
        }
        descriptions.want(&[AppId(500)]);

        descriptions.keep_only(&[AppId(1)]);

        assert!(descriptions.get(AppId(1)).is_some());
        assert!(descriptions.get(AppId(500)).is_some());
        assert!(descriptions.get(AppId(2)).is_none());
    }

    #[test]
    fn ignores_a_description_for_a_forgotten_game() {
        let mut descriptions = Descriptions::default();

        descriptions.loaded(AppId(1), blocks());

        assert!(descriptions.get(AppId(1)).is_none());
    }
}
