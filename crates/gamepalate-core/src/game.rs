use crate::{
    ids::{AppId, TagId},
    timestamp::Timestamp,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub app_id: AppId,
    pub name: String,
    pub price: Price,
    pub reviews: Reviews,
    pub is_early_access: bool,
    pub tags: Vec<GameTag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct Price {
    pub original_cents: u32,
    pub discount: Option<Discount>,
}

impl Price {
    pub fn final_cents(&self) -> u32 {
        match self.discount {
            Some(discount) => discount.final_cents,
            None => self.original_cents,
        }
    }

    pub fn as_of(self, now: Timestamp) -> Price {
        match self.discount {
            Some(Discount { end: Some(end), .. }) if end <= now => Price {
                discount: None,
                ..self
            },
            _ => self,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct Discount {
    pub percent: u8,
    pub final_cents: u32,
    pub end: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct Reviews {
    pub total: u32,
    pub positive: u32,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct GameTag {
    pub id: TagId,
    pub weight: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price_with_discount(end: Option<Timestamp>) -> Price {
        Price {
            original_cents: 2000,
            discount: Some(Discount {
                percent: 50,
                final_cents: 1000,
                end,
            }),
        }
    }

    #[test]
    fn as_of_without_discount_returns_same_price() {
        let price_without_discount = Price {
            original_cents: 2000,
            discount: None,
        };
        assert_eq!(
            price_without_discount.as_of(Timestamp(100)),
            price_without_discount
        );
    }

    #[test]
    fn as_of_removes_expired_discount() {
        let price_with_discount = price_with_discount(Some(Timestamp(50)));
        let result = price_with_discount.as_of(Timestamp(100));
        assert_eq!(result.discount, None);
        assert_eq!(result.final_cents(), 2000);
    }

    #[test]
    fn as_of_keeps_active_discount() {
        let price = price_with_discount(Some(Timestamp(200)));
        assert_eq!(price.as_of(Timestamp(100)), price);
    }
}
