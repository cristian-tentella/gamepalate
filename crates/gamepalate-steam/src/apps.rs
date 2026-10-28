use gamepalate_core::ids::AppId;
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const ENDPOINT: &str = "IStoreService/GetAppList/v1";
const PAGE_SIZE: &str = "50000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppListEntry {
    pub id: AppId,
    pub last_modified: u64,
    pub price_change_number: u64,
}

#[derive(Deserialize)]
struct AppListEnvelope {
    response: AppListResponse,
}

#[derive(Deserialize)]
struct AppListResponse {
    #[serde(default)]
    apps: Vec<SteamApp>,
    #[serde(default)]
    have_more_results: bool,
    last_appid: Option<u32>,
}

#[derive(Deserialize)]
struct SteamApp {
    appid: u32,
    last_modified: u64,
    price_change_number: u64,
}

#[derive(Debug)]
struct AppListPage {
    entries: Vec<AppListEntry>,
    next: Option<u32>,
}

pub async fn get_app_list<S: Get>(transport: &S) -> Result<Vec<AppListEntry>, SteamError> {
    let mut entries = Vec::new();
    let mut last_appid: Option<u32> = None;

    for page_number in 1.. {
        let started = std::time::Instant::now();
        let cursor = last_appid.map(|appid| appid.to_string());
        let mut params = vec![("max_results", PAGE_SIZE)];
        if let Some(cursor) = &cursor {
            params.push(("last_appid", cursor));
        }

        let value = transport.get(ENDPOINT, &params, true).await?;
        let page = parse_app_list_page(value)?;
        log::info!(
            "GetAppList page {page_number}: {} apps in {:.1?}",
            page.entries.len(),
            started.elapsed()
        );
        entries.extend(page.entries);

        match page.next {
            Some(next) => last_appid = Some(next),
            None => break,
        }
    }

    Ok(entries)
}

fn parse_app_list_page(value: Value) -> Result<AppListPage, SteamError> {
    let response = parse::<AppListEnvelope>(ENDPOINT, value)?.response;

    let entries = response
        .apps
        .into_iter()
        .map(|app| AppListEntry {
            id: AppId(app.appid),
            last_modified: app.last_modified,
            price_change_number: app.price_change_number,
        })
        .collect();

    let next = if response.have_more_results {
        response.last_appid
    } else {
        None
    };

    Ok(AppListPage { entries, next })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_apps_of_a_page() {
        let value = json!({
            "response": {
                "apps": [{ "appid": 10, "name": "Counter-Strike", "last_modified": 1700000000, "price_change_number": 21000000 }],
                "have_more_results": true,
                "last_appid": 10
            }
        });

        let page = parse_app_list_page(value).unwrap();

        assert_eq!(
            page.entries,
            vec![AppListEntry {
                id: AppId(10),
                last_modified: 1_700_000_000,
                price_change_number: 21_000_000,
            }]
        );
        assert_eq!(page.next, Some(10));
    }

    #[test]
    fn the_last_page_has_no_next() {
        let value = json!({ "response": { "apps": [], "last_appid": 10 } });

        let page = parse_app_list_page(value).unwrap();

        assert_eq!(page.next, None);
    }

    #[test]
    fn more_results_without_a_last_appid_stops() {
        let value = json!({ "response": { "have_more_results": true } });

        let page = parse_app_list_page(value).unwrap();

        assert!(page.entries.is_empty());
        assert_eq!(page.next, None);
    }

    #[test]
    fn a_missing_field_is_a_shape_error() {
        let value = json!({ "response": { "apps": [{ "appid": 10, "last_modified": 1 }] } });

        let error = parse_app_list_page(value).unwrap_err();

        assert!(matches!(error, SteamError::Shape { .. }));
        assert!(error.to_string().starts_with(ENDPOINT));
    }
}
