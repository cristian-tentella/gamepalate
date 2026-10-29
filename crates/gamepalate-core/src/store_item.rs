use crate::{
    game::{Discount, Game, GameTag, Price, Reviews},
    ids::{AppId, TagId},
    off_catalog::OffCatalogGame,
    timestamp::Timestamp,
};

const GAME_TYPE: u32 = 0;

#[derive(Debug, Clone, PartialEq)]
pub struct StoreItem {
    pub id: AppId,
    pub name: String,
    pub app_type: u32,
    pub is_free: bool,
    pub is_early_access: bool,
    pub is_coming_soon: bool,
    pub release_date: Option<Timestamp>,
    pub price: Option<ItemPrice>,
    pub reviews: Reviews,
    pub tags: Vec<ItemTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPrice {
    pub final_cents: u32,
    pub original_cents: u32,
    pub discount_percent: u8,
    pub discount_end: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemTag {
    pub id: TagId,
    pub weight: f64,
}

impl StoreItem {
    pub fn into_game(self, now: Timestamp) -> Option<Game> {
        let price = self.price?;
        if self.app_type != GAME_TYPE || self.is_free || price.original_cents == 0 {
            return None;
        }
        if self.is_coming_soon || self.release_date.is_some_and(|date| date > now) {
            return None;
        }

        Some(Game {
            id: self.id,
            name: self.name,
            price: price.to_price().as_of(now),
            reviews: self.reviews,
            is_early_access: self.is_early_access,
            tags: normalize(self.tags),
        })
    }
}

impl StoreItem {
    pub fn into_off_catalog_game(self) -> OffCatalogGame {
        OffCatalogGame {
            tags: normalize(self.tags),
            name: Some(self.name),
        }
    }
}

impl ItemPrice {
    fn to_price(self) -> Price {
        let discount = (self.discount_percent > 0).then_some(Discount {
            percent: self.discount_percent,
            final_cents: self.final_cents,
            end: self.discount_end,
        });

        Price {
            original_cents: self.original_cents,
            discount,
        }
    }
}

fn normalize(tags: Vec<ItemTag>) -> Vec<GameTag> {
    let total: f64 = tags.iter().map(|tag| tag.weight).sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let mut normalized: Vec<GameTag> = tags
        .into_iter()
        .map(|tag| GameTag {
            id: tag.id,
            weight: tag.weight / total,
        })
        .collect();
    normalized.sort_unstable_by_key(|tag| tag.id);
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn item() -> StoreItem {
        StoreItem {
            id: AppId(620),
            name: "Portal 2".to_string(),
            app_type: GAME_TYPE,
            is_free: false,
            is_early_access: false,
            is_coming_soon: false,
            release_date: Some(Timestamp(1_303_186_800_000)),
            price: Some(ItemPrice {
                final_cents: 969,
                original_cents: 969,
                discount_percent: 0,
                discount_end: None,
            }),
            reviews: Reviews {
                total: 100,
                positive: 98,
            },
            tags: vec![
                ItemTag {
                    id: TagId(19),
                    weight: 100.0,
                },
                ItemTag {
                    id: TagId(5),
                    weight: 300.0,
                },
            ],
        }
    }

    fn discounted(end: Option<Timestamp>) -> StoreItem {
        StoreItem {
            price: Some(ItemPrice {
                final_cents: 500,
                original_cents: 1000,
                discount_percent: 50,
                discount_end: end,
            }),
            ..item()
        }
    }

    #[test]
    fn a_paid_released_game_becomes_a_game() {
        let game = item().into_game(NOW).unwrap();

        assert_eq!(game.id, AppId(620));
        assert_eq!(game.price.final_cents(), 969);
        assert_eq!(game.price.discount, None);
        assert_eq!(game.reviews.positive, 98);
    }

    #[test]
    fn tags_are_normalized_and_sorted_by_id() {
        let game = item().into_game(NOW).unwrap();

        assert_eq!(
            game.tags,
            [
                GameTag {
                    id: TagId(5),
                    weight: 0.75
                },
                GameTag {
                    id: TagId(19),
                    weight: 0.25
                },
            ]
        );
    }

    #[test]
    fn tags_with_zero_total_weight_are_dropped() {
        let item = StoreItem {
            tags: vec![ItemTag {
                id: TagId(19),
                weight: 0.0,
            }],
            ..item()
        };

        assert!(item.into_game(NOW).unwrap().tags.is_empty());
    }

    #[test]
    fn an_active_discount_is_kept() {
        let end = Timestamp(NOW.0 + 1000);
        let game = discounted(Some(end)).into_game(NOW).unwrap();

        assert_eq!(
            game.price.discount,
            Some(Discount {
                percent: 50,
                final_cents: 500,
                end: Some(end),
            })
        );
    }

    #[test]
    fn an_expired_discount_is_removed() {
        let game = discounted(Some(Timestamp(NOW.0 - 1000)))
            .into_game(NOW)
            .unwrap();

        assert_eq!(game.price.discount, None);
        assert_eq!(game.price.final_cents(), 1000);
    }

    #[test]
    fn a_full_discount_stays_a_game() {
        let item = StoreItem {
            price: Some(ItemPrice {
                final_cents: 0,
                original_cents: 1000,
                discount_percent: 100,
                discount_end: None,
            }),
            ..item()
        };

        assert_eq!(item.into_game(NOW).unwrap().price.final_cents(), 0);
    }

    #[test]
    fn not_a_game_type_is_left_out() {
        let item = StoreItem {
            app_type: 1,
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn free_is_left_out() {
        let item = StoreItem {
            is_free: true,
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn without_a_price_is_left_out() {
        let item = StoreItem {
            price: None,
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn original_price_zero_is_left_out() {
        let item = StoreItem {
            price: Some(ItemPrice {
                final_cents: 0,
                original_cents: 0,
                discount_percent: 0,
                discount_end: None,
            }),
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn coming_soon_is_left_out() {
        let item = StoreItem {
            is_coming_soon: true,
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn released_after_now_is_left_out() {
        let item = StoreItem {
            release_date: Some(Timestamp(NOW.0 + 1)),
            ..item()
        };
        assert_eq!(item.into_game(NOW), None);
    }

    #[test]
    fn early_access_without_a_release_date_stays_a_game() {
        let item = StoreItem {
            is_early_access: true,
            release_date: None,
            ..item()
        };
        assert!(item.into_game(NOW).unwrap().is_early_access);
    }
}
