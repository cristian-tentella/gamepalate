use std::collections::HashMap;

use gamepalate_core::{ids::AppId, timestamp::Timestamp};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    images::ASSETS_BASE_URL,
    transport::{Get, SteamError, parse},
};

const ITEMS_ENDPOINT: &str = "IStoreBrowseService/GetItems/v1";
const CATEGORIES_ENDPOINT: &str = "IStoreBrowseService/GetStoreCategories/v1";
const SUCCESS: u32 = 1;
const ITALIAN: u32 = 3;
const FULL_CONTROLLER: u32 = 28;
const PARTIAL_CONTROLLER: u32 = 18;

#[derive(Debug, Clone, PartialEq)]
pub struct ItemDetails {
    pub short_description: Option<String>,
    pub developers: Vec<String>,
    pub release_date: Option<Timestamp>,
    pub header_url: Option<String>,
    pub screenshot_urls: Vec<String>,
    pub player_categories: Vec<u32>,
    pub controller: Option<Controller>,
    pub italian: Option<Italian>,
    pub steam_deck: Option<SteamDeck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Controller {
    Full,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Italian {
    pub subtitles: bool,
    pub audio: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamDeck {
    Unsupported,
    Playable,
    Verified,
}

#[derive(Deserialize)]
struct DetailsEnvelope {
    response: DetailsResponse,
}

#[derive(Deserialize)]
struct DetailsResponse {
    #[serde(default)]
    store_items: Vec<SteamDetails>,
}

#[derive(Deserialize)]
struct SteamDetails {
    success: u32,
    basic_info: Option<BasicInfo>,
    release: Option<Release>,
    assets: Option<Assets>,
    screenshots: Option<Screenshots>,
    #[serde(default)]
    categories: Categories,
    #[serde(default)]
    supported_languages: Vec<Language>,
    platforms: Option<Platforms>,
}

#[derive(Deserialize)]
struct BasicInfo {
    short_description: Option<String>,
    #[serde(default)]
    developers: Vec<Named>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Release {
    steam_release_date: Option<i64>,
}

#[derive(Deserialize)]
struct Assets {
    asset_url_format: Option<String>,
    header: Option<String>,
}

#[derive(Deserialize)]
struct Screenshots {
    #[serde(default)]
    all_ages_screenshots: Vec<ScreenshotFile>,
}

#[derive(Deserialize)]
struct ScreenshotFile {
    filename: String,
}

#[derive(Default, Deserialize)]
struct Categories {
    #[serde(default)]
    supported_player_categoryids: Vec<u32>,
    #[serde(default)]
    controller_categoryids: Vec<u32>,
}

#[derive(Deserialize)]
struct Language {
    elanguage: u32,
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    subtitles: bool,
    #[serde(default)]
    full_audio: bool,
}

#[derive(Deserialize)]
struct Platforms {
    steam_deck_compat_category: Option<u32>,
}

#[derive(Deserialize)]
struct CategoriesEnvelope {
    response: CategoriesResponse,
}

#[derive(Deserialize)]
struct CategoriesResponse {
    #[serde(default)]
    categories: Vec<StoreCategory>,
}

#[derive(Deserialize)]
struct StoreCategory {
    categoryid: u32,
    display_name: String,
}

pub async fn get_item_details<S: Get>(
    transport: &S,
    id: AppId,
    language: &str,
    country_code: &str,
) -> Result<Option<ItemDetails>, SteamError> {
    let input = json!({
        "ids": [{ "appid": id.0 }],
        "context": { "language": language, "country_code": country_code },
        "data_request": {
            "include_assets": true,
            "include_basic_info": true,
            "include_release": true,
            "include_screenshots": true,
            "include_platforms": true,
            "include_supported_languages": true,
        },
    })
    .to_string();

    let value = transport
        .get(ITEMS_ENDPOINT, &[("input_json", &input)], false)
        .await?;
    parse_details(value)
}

pub async fn get_store_categories<S: Get>(
    transport: &S,
    language: &str,
) -> Result<HashMap<u32, String>, SteamError> {
    let value = transport
        .get(CATEGORIES_ENDPOINT, &[("language", language)], false)
        .await?;
    parse_categories(value)
}

fn parse_details(value: Value) -> Result<Option<ItemDetails>, SteamError> {
    let envelope: DetailsEnvelope = parse(ITEMS_ENDPOINT, value)?;

    Ok(envelope
        .response
        .store_items
        .into_iter()
        .next()
        .filter(|item| item.success == SUCCESS)
        .map(to_details))
}

fn parse_categories(value: Value) -> Result<HashMap<u32, String>, SteamError> {
    let envelope: CategoriesEnvelope = parse(CATEGORIES_ENDPOINT, value)?;

    Ok(envelope
        .response
        .categories
        .into_iter()
        .map(|category| (category.categoryid, category.display_name))
        .collect())
}

fn to_details(item: SteamDetails) -> ItemDetails {
    let (short_description, developers) = match item.basic_info {
        Some(info) => (
            info.short_description
                .filter(|description| !description.is_empty()),
            info.developers
                .into_iter()
                .map(|named| named.name)
                .collect(),
        ),
        None => (None, Vec::new()),
    };
    let controllers = &item.categories.controller_categoryids;
    let controller = if controllers.contains(&FULL_CONTROLLER) {
        Some(Controller::Full)
    } else if controllers.contains(&PARTIAL_CONTROLLER) {
        Some(Controller::Partial)
    } else {
        None
    };

    ItemDetails {
        short_description,
        developers,
        release_date: item
            .release
            .and_then(|release| release.steam_release_date)
            .map(|seconds| Timestamp(seconds * 1000)),
        header_url: item.assets.and_then(header_url),
        screenshot_urls: item
            .screenshots
            .map(|screenshots| {
                screenshots
                    .all_ages_screenshots
                    .into_iter()
                    .map(|file| format!("{ASSETS_BASE_URL}{}", file.filename))
                    .collect()
            })
            .unwrap_or_default(),
        controller,
        player_categories: item.categories.supported_player_categoryids,
        italian: item
            .supported_languages
            .iter()
            .find(|language| language.elanguage == ITALIAN && language.supported)
            .map(|language| Italian {
                subtitles: language.subtitles,
                audio: language.full_audio,
            }),
        steam_deck: match item
            .platforms
            .and_then(|platforms| platforms.steam_deck_compat_category)
        {
            Some(1) => Some(SteamDeck::Unsupported),
            Some(2) => Some(SteamDeck::Playable),
            Some(3) => Some(SteamDeck::Verified),
            _ => None,
        },
    }
}

fn header_url(assets: Assets) -> Option<String> {
    let format = assets.asset_url_format?;
    let header = assets.header?;

    Some(format!(
        "{ASSETS_BASE_URL}{}",
        format.replace("${FILENAME}", &header)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(item: Value) -> Option<ItemDetails> {
        parse_details(json!({ "response": { "store_items": [item] } })).unwrap()
    }

    #[test]
    fn reads_a_complete_item() {
        let details = one(json!({
            "id": 1_369_630,
            "success": 1,
            "basic_info": {
                "short_description": "A gothic action RPG.",
                "developers": [{ "name": "Live Wire" }, { "name": "Adglobe" }],
                "publishers": [{ "name": "Binary Haze" }],
            },
            "release": { "steam_release_date": 1_624_233_600 },
            "assets": {
                "asset_url_format": "steam/apps/1369630/abc123/${FILENAME}?t=1",
                "header": "header.jpg",
            },
            "screenshots": { "all_ages_screenshots": [
                { "filename": "steam/apps/1369630/ss_1.jpg", "ordinal": 0 },
                { "filename": "steam/apps/1369630/ss_2.jpg", "ordinal": 1 },
            ] },
            "categories": {
                "supported_player_categoryids": [2],
                "controller_categoryids": [18, 28],
            },
            "supported_languages": [
                { "elanguage": 0, "supported": true, "full_audio": true },
                { "elanguage": 3, "supported": true, "subtitles": true },
            ],
            "platforms": { "steam_deck_compat_category": 3 },
        }));

        assert_eq!(
            details,
            Some(ItemDetails {
                short_description: Some("A gothic action RPG.".to_string()),
                developers: vec!["Live Wire".to_string(), "Adglobe".to_string()],
                release_date: Some(Timestamp(1_624_233_600_000)),
                header_url: Some(format!(
                    "{ASSETS_BASE_URL}steam/apps/1369630/abc123/header.jpg?t=1"
                )),
                screenshot_urls: vec![
                    format!("{ASSETS_BASE_URL}steam/apps/1369630/ss_1.jpg"),
                    format!("{ASSETS_BASE_URL}steam/apps/1369630/ss_2.jpg"),
                ],
                player_categories: vec![2],
                controller: Some(Controller::Full),
                italian: Some(Italian {
                    subtitles: true,
                    audio: false,
                }),
                steam_deck: Some(SteamDeck::Verified),
            })
        );
    }

    #[test]
    fn missing_fields_are_empty() {
        let details = one(json!({ "id": 620, "success": 1 }));

        assert_eq!(
            details,
            Some(ItemDetails {
                short_description: None,
                developers: Vec::new(),
                release_date: None,
                header_url: None,
                screenshot_urls: Vec::new(),
                player_categories: Vec::new(),
                controller: None,
                italian: None,
                steam_deck: None,
            })
        );
    }

    #[test]
    fn an_empty_description_is_none() {
        let details = one(json!({
            "success": 1,
            "basic_info": { "short_description": "" },
        }));

        assert_eq!(details.unwrap().short_description, None);
    }

    #[test]
    fn partial_controller_and_playable_deck() {
        let details = one(json!({
            "success": 1,
            "categories": { "controller_categoryids": [18] },
            "platforms": { "steam_deck_compat_category": 2 },
        }))
        .unwrap();

        assert_eq!(details.controller, Some(Controller::Partial));
        assert_eq!(details.steam_deck, Some(SteamDeck::Playable));
    }

    #[test]
    fn italian_listed_but_not_supported_is_none() {
        let details = one(json!({
            "success": 1,
            "supported_languages": [{ "elanguage": 3, "supported": false, "subtitles": true }],
            "platforms": { "steam_deck_compat_category": 0 },
        }))
        .unwrap();

        assert_eq!(details.italian, None);
        assert_eq!(details.steam_deck, None);
    }

    #[test]
    fn an_item_without_a_store_page_is_none() {
        assert_eq!(one(json!({ "id": 3, "success": 15 })), None);
        assert_eq!(parse_details(json!({ "response": {} })).unwrap(), None);
    }

    #[test]
    fn a_wrong_shape_is_an_error() {
        let result =
            parse_details(json!({ "response": { "store_items": [{ "success": "yes" }] } }));

        assert!(matches!(result, Err(SteamError::Shape { .. })));
    }

    #[test]
    fn reads_the_category_names() {
        let names = parse_categories(json!({ "response": { "categories": [
            { "categoryid": 2, "type": 0, "display_name": "Single-player" },
            { "categoryid": 38, "type": 0, "display_name": "Online Co-op" },
        ] } }))
        .unwrap();

        assert_eq!(names.len(), 2);
        assert_eq!(names[&2], "Single-player");
        assert_eq!(names[&38], "Online Co-op");
    }
}
