use std::collections::{HashMap, HashSet};

use crate::{game::Game, ids::AppId, store_item::StoreItem, timestamp::Timestamp};

pub fn changed_apps(
    current: impl IntoIterator<Item = (AppId, u64)>,
    previous: &HashMap<AppId, u64>,
) -> Vec<AppId> {
    current
        .into_iter()
        .filter(|(id, price_change_number)| previous.get(id) != Some(price_change_number))
        .map(|(id, _)| id)
        .collect()
}

pub struct KnownApps {
    listed: HashSet<AppId>,
    now: Timestamp,
    decided: HashMap<AppId, Option<Game>>,
}

impl KnownApps {
    pub fn new(listed: impl IntoIterator<Item = AppId>, now: Timestamp) -> KnownApps {
        KnownApps {
            listed: listed.into_iter().collect(),
            now,
            decided: HashMap::new(),
        }
    }

    pub fn add(&mut self, asked: &[AppId], items: Vec<StoreItem>) {
        let mut found: HashMap<AppId, StoreItem> =
            items.into_iter().map(|item| (item.id, item)).collect();

        for id in asked {
            if !self.listed.contains(id) || self.decided.contains_key(id) {
                continue;
            }
            let game = found.remove(id).and_then(|item| item.into_game(self.now));
            self.decided.insert(*id, game);
        }
    }

    pub fn into_games(self) -> Vec<Game> {
        self.decided.into_values().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        game::Reviews,
        ids::TagId,
        store_item::{ItemPrice, ItemTag},
    };

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    fn item(id: u32, name: &str) -> StoreItem {
        StoreItem {
            id: AppId(id),
            name: name.to_string(),
            app_type: 0,
            is_free: false,
            is_early_access: false,
            is_coming_soon: false,
            release_date: Some(Timestamp(NOW.0 - 1)),
            price: Some(ItemPrice {
                final_cents: 999,
                original_cents: 999,
                discount_percent: 0,
                discount_end: None,
            }),
            formatted_price: Some("9,99€".to_string()),
            reviews: Reviews {
                total: 10,
                positive: 9,
            },
            tags: vec![ItemTag {
                id: TagId(19),
                weight: 1.0,
            }],
        }
    }

    fn names(known: KnownApps) -> Vec<(u32, String)> {
        let mut games: Vec<(u32, String)> = known
            .into_games()
            .into_iter()
            .map(|game| (game.id.0, game.name))
            .collect();
        games.sort();
        games
    }

    #[test]
    fn new_apps_and_changed_price_change_numbers_are_asked() {
        let previous = HashMap::from([(AppId(1), 10), (AppId(2), 20)]);

        let changed = changed_apps([(AppId(1), 10), (AppId(2), 21), (AppId(3), 30)], &previous);

        assert_eq!(changed, [AppId(2), AppId(3)]);
    }

    #[test]
    fn the_first_answer_about_an_app_decides_it() {
        let mut known = KnownApps::new([AppId(1)], NOW);

        known.add(&[AppId(1)], vec![item(1, "New")]);
        known.add(&[AppId(1)], vec![item(1, "Old")]);

        assert_eq!(names(known), [(1, "New".to_string())]);
    }

    #[test]
    fn an_app_steam_no_longer_had_stays_out_even_if_older_answers_had_it() {
        let mut known = KnownApps::new([AppId(1)], NOW);

        known.add(&[AppId(1)], vec![]);
        known.add(&[AppId(1)], vec![item(1, "Old")]);

        assert!(names(known).is_empty());
    }

    #[test]
    fn apps_no_longer_listed_are_left_out() {
        let mut known = KnownApps::new([AppId(1)], NOW);

        known.add(
            &[AppId(1), AppId(2)],
            vec![item(1, "Kept"), item(2, "Delisted")],
        );

        assert_eq!(names(known), [(1, "Kept".to_string())]);
    }

    #[test]
    fn games_are_classified_with_the_new_now() {
        let mut upcoming = item(1, "Upcoming");
        upcoming.release_date = Some(Timestamp(NOW.0 + 1000));
        let mut ended = item(2, "Discount ended");
        ended.price = Some(ItemPrice {
            final_cents: 499,
            original_cents: 999,
            discount_percent: 50,
            discount_end: Some(Timestamp(NOW.0 + 1000)),
        });
        let later = Timestamp(NOW.0 + 2000);
        let mut known = KnownApps::new([AppId(1), AppId(2)], later);

        known.add(&[AppId(1), AppId(2)], vec![upcoming, ended]);

        let games = known.into_games();
        assert_eq!(games.len(), 2);
        let ended = games.iter().find(|game| game.id == AppId(2)).unwrap();
        assert_eq!(ended.price.discount, None);
    }
}
