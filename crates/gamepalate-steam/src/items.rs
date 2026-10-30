use std::{fmt::Display, str::FromStr};

use futures::{Stream, StreamExt, stream};
use gamepalate_core::{
    game::Reviews,
    ids::{AppId, TagId},
    store_item::{ItemPrice, ItemTag, StoreItem},
    timestamp::Timestamp,
};
use serde::{Deserialize, Deserializer, de};
use serde_json::{Value, json};

use crate::{
    archive::{ArchivedRecord, ArchivedUpdate},
    transport::{Get, SteamError, parse},
};

const ENDPOINT: &str = "IStoreBrowseService/GetItems/v1";
const TAG_COUNT: u32 = 20;
const SUCCESS: u32 = 1;
pub const BATCH_SIZE: usize = 260;

pub struct ArchivedBatch {
    pub asked: Vec<AppId>,
    pub items: Vec<StoreItem>,
}

#[derive(Deserialize)]
struct ItemsInput {
    ids: Vec<InputId>,
}

#[derive(Deserialize)]
struct InputId {
    appid: u32,
}

#[derive(Deserialize)]
struct ItemsEnvelope {
    response: ItemsResponse,
}

#[derive(Deserialize)]
struct ItemsResponse {
    #[serde(default)]
    store_items: Vec<SteamItem>,
}

#[derive(Deserialize)]
struct SteamItem {
    id: u32,
    success: u32,
    name: Option<String>,
    #[serde(rename = "type")]
    app_type: Option<u32>,
    #[serde(default)]
    is_free: bool,
    #[serde(default)]
    is_coming_soon: bool,
    #[serde(default)]
    is_early_access: bool,
    reviews: Option<SteamReviews>,
    #[serde(default)]
    tags: Vec<SteamTag>,
    release: Option<SteamRelease>,
    best_purchase_option: Option<PurchaseOption>,
    #[serde(default)]
    purchase_options: Vec<PurchaseOption>,
}

#[derive(Deserialize)]
struct SteamReviews {
    summary_filtered: Option<ReviewSummary>,
}

#[derive(Deserialize)]
struct ReviewSummary {
    review_count: u32,
    percent_positive: u32,
}

#[derive(Deserialize)]
struct SteamTag {
    tagid: u32,
    weight: f64,
}

#[derive(Deserialize)]
struct SteamRelease {
    steam_release_date: Option<i64>,
    #[serde(default)]
    is_coming_soon: bool,
}

#[derive(Deserialize)]
struct PurchaseOption {
    packageid: Option<u32>,
    bundleid: Option<u32>,
    final_price_in_cents: Option<Int<u32>>,
    original_price_in_cents: Option<Int<u32>>,
    discount_pct: Option<u8>,
    formatted_final_price: Option<String>,
    #[serde(default)]
    active_discounts: Vec<ActiveDiscount>,
}

#[derive(Deserialize)]
struct ActiveDiscount {
    discount_end_date: Option<Int<i64>>,
}

#[derive(Debug, Clone, Copy)]
struct Int<T>(T);

impl<'de, T> Deserialize<'de> for Int<T>
where
    T: Deserialize<'de> + FromStr,
    T::Err: Display,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NumberOrText<T> {
            Number(T),
            Text(String),
        }

        match NumberOrText::<T>::deserialize(deserializer)? {
            NumberOrText::Number(number) => Ok(Int(number)),
            NumberOrText::Text(text) => text.parse().map(Int).map_err(de::Error::custom),
        }
    }
}

pub fn batch_count(ids: usize) -> usize {
    ids.div_ceil(BATCH_SIZE)
}

pub fn item_batches<'a, S: Get>(
    transport: &'a S,
    ids: &'a [AppId],
    language: &'a str,
    country_code: &'a str,
    concurrency: usize,
) -> impl Stream<Item = Result<Vec<StoreItem>, SteamError>> + 'a {
    stream::iter(ids.chunks(BATCH_SIZE))
        .map(move |batch| get_items(transport, batch, language, country_code))
        .buffer_unordered(concurrency)
}

pub async fn get_items<S: Get>(
    transport: &S,
    ids: &[AppId],
    language: &str,
    country_code: &str,
) -> Result<Vec<StoreItem>, SteamError> {
    assert!(
        ids.len() <= BATCH_SIZE,
        "at most {BATCH_SIZE} appids per GetItems"
    );

    let input = json!({
        "ids": ids.iter().map(|id| json!({ "appid": id.0 })).collect::<Vec<_>>(),
        "context": { "language": language, "country_code": country_code },
        "data_request": {
            "include_tag_count": TAG_COUNT,
            "include_basic_info": true,
            "include_release": true,
            "include_reviews": true,
            "include_all_purchase_options": true,
        },
    })
    .to_string();

    let value = transport
        .get(ENDPOINT, &[("input_json", &input)], false)
        .await?;
    parse_items(value)
}

pub fn archived_batches(
    update: &ArchivedUpdate,
) -> Result<impl Iterator<Item = Result<ArchivedBatch, SteamError>>, SteamError> {
    Ok(update
        .records(ENDPOINT)?
        .map(|record| record.and_then(asked_and_items)))
}

fn asked_and_items(record: ArchivedRecord) -> Result<ArchivedBatch, SteamError> {
    let shape = |reason: String| SteamError::Shape {
        endpoint: ENDPOINT.to_string(),
        reason,
    };
    let input = record
        .params
        .get("input_json")
        .and_then(Value::as_str)
        .ok_or_else(|| shape("the archived request has no input_json".to_string()))?;
    let input: ItemsInput = serde_json::from_str(input)
        .map_err(|error| shape(format!("the archived input_json: {error}")))?;

    let asked = input.ids.into_iter().map(|id| AppId(id.appid)).collect();
    Ok(ArchivedBatch {
        asked,
        items: parse_items(record.body)?,
    })
}

fn parse_items(value: Value) -> Result<Vec<StoreItem>, SteamError> {
    let envelope: ItemsEnvelope = parse(ENDPOINT, value)?;

    envelope
        .response
        .store_items
        .into_iter()
        .enumerate()
        .filter(|(_, item)| item.success == SUCCESS)
        .map(|(index, item)| to_store_item(index, item))
        .collect()
}

fn to_store_item(index: usize, item: SteamItem) -> Result<StoreItem, SteamError> {
    let missing = |field: &str| SteamError::Shape {
        endpoint: ENDPOINT.to_string(),
        reason: format!("response.store_items[{index}].{field} is missing"),
    };
    let name = item.name.ok_or_else(|| missing("name"))?;
    let app_type = item.app_type.ok_or_else(|| missing("type"))?;

    let release = item.release;
    let release_coming_soon = release
        .as_ref()
        .is_some_and(|release| release.is_coming_soon);
    let release_date = release
        .and_then(|release| release.steam_release_date)
        .map(seconds);

    let option = game_option(item.best_purchase_option, item.purchase_options);

    Ok(StoreItem {
        id: AppId(item.id),
        name,
        app_type,
        is_free: item.is_free,
        is_early_access: item.is_early_access,
        is_coming_soon: item.is_coming_soon || release_coming_soon,
        release_date,
        price: option.as_ref().and_then(item_price),
        formatted_price: option.and_then(|option| option.formatted_final_price),
        reviews: reviews(item.reviews.and_then(|reviews| reviews.summary_filtered)),
        tags: item
            .tags
            .into_iter()
            .map(|tag| ItemTag {
                id: TagId(tag.tagid),
                weight: tag.weight,
            })
            .collect(),
    })
}

fn game_option(
    best: Option<PurchaseOption>,
    options: Vec<PurchaseOption>,
) -> Option<PurchaseOption> {
    if let Some(best) = best
        && best.bundleid.is_none()
    {
        return Some(best);
    }

    options
        .into_iter()
        .filter(|option| option.packageid.is_some() && option.bundleid.is_none())
        .filter_map(|option| option.final_price_in_cents.map(|price| (price.0, option)))
        .min_by_key(|(price, _)| *price)
        .map(|(_, option)| option)
}

fn item_price(option: &PurchaseOption) -> Option<ItemPrice> {
    let final_cents = option.final_price_in_cents?.0;

    Some(ItemPrice {
        final_cents,
        original_cents: option
            .original_price_in_cents
            .map_or(final_cents, |price| price.0),
        discount_percent: option.discount_pct.unwrap_or(0),
        discount_end: option
            .active_discounts
            .iter()
            .find_map(|discount| discount.discount_end_date)
            .map(|end| seconds(end.0)),
    })
}

fn reviews(summary: Option<ReviewSummary>) -> Reviews {
    let Some(summary) = summary else {
        return Reviews {
            total: 0,
            positive: 0,
        };
    };

    let positive = (f64::from(summary.review_count) * f64::from(summary.percent_positive) / 100.0)
        .round() as u32;
    Reviews {
        total: summary.review_count,
        positive,
    }
}

fn seconds(seconds: i64) -> Timestamp {
    Timestamp(seconds * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(fields: Value) -> Value {
        let mut item = json!({ "id": 620, "success": 1, "name": "Portal 2", "type": 0 });
        item.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        item
    }

    fn one(item: Value) -> StoreItem {
        let mut items = parse_items(json!({ "response": { "store_items": [item] } })).unwrap();
        assert_eq!(items.len(), 1);
        items.remove(0)
    }

    #[test]
    fn reads_a_discounted_item() {
        let item = one(item(json!({
            "is_early_access": true,
            "release": { "steam_release_date": 1_303_186_800 },
            "reviews": { "summary_filtered": { "review_count": 3, "percent_positive": 67 } },
            "tags": [{ "tagid": 19, "weight": 604 }],
            "best_purchase_option": {
                "packageid": 7877,
                "final_price_in_cents": "1119",
                "original_price_in_cents": "1399",
                "discount_pct": 20,
                "formatted_final_price": "11,19€",
                "active_discounts": [{ "discount_end_date": 1_793_638_800 }],
            },
            "unknown": "ignored",
        })));

        assert_eq!(
            item,
            StoreItem {
                id: AppId(620),
                name: "Portal 2".to_string(),
                app_type: 0,
                is_free: false,
                is_early_access: true,
                is_coming_soon: false,
                release_date: Some(Timestamp(1_303_186_800_000)),
                price: Some(ItemPrice {
                    final_cents: 1119,
                    original_cents: 1399,
                    discount_percent: 20,
                    discount_end: Some(Timestamp(1_793_638_800_000)),
                }),
                formatted_price: Some("11,19€".to_string()),
                reviews: Reviews {
                    total: 3,
                    positive: 2,
                },
                tags: vec![ItemTag {
                    id: TagId(19),
                    weight: 604.0,
                }],
            }
        );
    }

    #[test]
    fn prices_can_be_numbers() {
        let item = one(item(json!({
            "best_purchase_option": { "packageid": 1, "final_price_in_cents": 969 },
        })));

        assert_eq!(item.price.unwrap().final_cents, 969);
        assert_eq!(item.price.unwrap().original_cents, 969);
    }

    #[test]
    fn a_bundle_as_best_option_takes_the_cheapest_package() {
        let item = one(item(json!({
            "best_purchase_option": { "bundleid": 234, "final_price_in_cents": "500" },
            "purchase_options": [
                { "packageid": 7877, "final_price_in_cents": "969" },
                { "packageid": 8187, "final_price_in_cents": "159" },
                { "bundleid": 234, "final_price_in_cents": "500" },
                { "packageid": 9000 },
            ],
        })));

        assert_eq!(item.price.unwrap().final_cents, 159);
    }

    #[test]
    fn the_formatted_price_is_the_one_of_the_chosen_package() {
        let item = one(item(json!({
            "best_purchase_option": {
                "bundleid": 234,
                "final_price_in_cents": "500",
                "formatted_final_price": "5,00€",
            },
            "purchase_options": [
                { "packageid": 7877, "final_price_in_cents": "969", "formatted_final_price": "9,69€" },
                { "packageid": 8187, "final_price_in_cents": "159", "formatted_final_price": "1,59€" },
                { "bundleid": 234, "final_price_in_cents": "500", "formatted_final_price": "5,00€" },
            ],
        })));

        assert_eq!(item.formatted_price.as_deref(), Some("1,59€"));
    }

    #[test]
    fn without_a_formatted_price_it_is_none() {
        let item = one(item(json!({
            "best_purchase_option": { "packageid": 1, "final_price_in_cents": 969 },
        })));

        assert_eq!(item.formatted_price, None);
    }

    #[test]
    fn only_bundles_means_no_price() {
        let item = one(item(json!({
            "best_purchase_option": { "bundleid": 234, "final_price_in_cents": "500" },
            "purchase_options": [{ "bundleid": 234, "final_price_in_cents": "500" }],
        })));

        assert_eq!(item.price, None);
    }

    #[test]
    fn coming_soon_comes_from_either_place() {
        let top = one(item(json!({ "is_coming_soon": true })));
        let release = one(item(json!({ "release": { "is_coming_soon": true } })));

        assert!(top.is_coming_soon && release.is_coming_soon);
    }

    #[test]
    fn missing_lists_and_reviews_are_empty() {
        let item = one(item(json!({})));

        assert!(item.tags.is_empty());
        assert_eq!(item.price, None);
        assert_eq!(
            item.reviews,
            Reviews {
                total: 0,
                positive: 0
            }
        );
    }

    #[test]
    fn unknown_items_are_skipped() {
        let items = parse_items(json!({ "response": { "store_items": [
            { "id": 3, "success": 15 },
            item(json!({})),
        ] } }))
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, AppId(620));
    }

    #[test]
    fn missing_store_items_is_empty() {
        assert!(parse_items(json!({ "response": {} })).unwrap().is_empty());
    }

    #[test]
    fn a_found_item_without_name_is_a_shape_error() {
        let result = parse_items(json!({ "response": { "store_items": [
            { "id": 3, "success": 15 },
            { "id": 620, "success": 1, "type": 0 },
        ] } }));

        match result {
            Err(SteamError::Shape { reason, .. }) => {
                assert_eq!(reason, "response.store_items[1].name is missing")
            }
            other => panic!("expected Shape, got {other:?}"),
        }
    }

    #[test]
    fn a_price_that_is_not_a_number_is_a_shape_error() {
        let result = parse_items(json!({ "response": { "store_items": [item(json!({
            "best_purchase_option": { "packageid": 1, "final_price_in_cents": "abc" },
        }))] } }));

        assert!(matches!(result, Err(SteamError::Shape { .. })));
    }

    fn record(params: Value, body: Value) -> ArchivedRecord {
        ArchivedRecord {
            params: params.as_object().unwrap().clone(),
            body,
        }
    }

    #[test]
    fn an_archived_batch_tells_what_it_asked_and_what_steam_had() {
        let input = json!({ "ids": [{ "appid": 620 }, { "appid": 3 }] }).to_string();
        let record = record(
            json!({ "input_json": input }),
            json!({ "response": { "store_items": [item(json!({})), { "id": 3, "success": 15 }] } }),
        );

        let batch = asked_and_items(record).unwrap();

        assert_eq!(batch.asked, [AppId(620), AppId(3)]);
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].id, AppId(620));
    }

    #[test]
    fn an_archived_batch_without_input_json_is_a_shape_error() {
        let record = record(json!({}), json!({ "response": {} }));

        assert!(matches!(
            asked_and_items(record),
            Err(SteamError::Shape { .. })
        ));
    }
}
