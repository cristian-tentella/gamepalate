use gamepalate_core::{
    ids::{AppId, SteamId},
    signals::{LibraryGame, RecentGame, SignalsReading},
    timestamp::Timestamp,
};
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const OWNED_GAMES: &str = "IPlayerService/GetOwnedGames/v1";
const RECENTLY_PLAYED: &str = "IPlayerService/GetRecentlyPlayedGames/v1";
const WISHLIST: &str = "IWishlistService/GetWishlist/v1";
const FOLLOWED: &str = "IStoreService/GetGamesFollowed/v1";

#[derive(Debug)]
pub struct SteamSignals {
    pub library: Vec<LibraryGame>,
    pub recent: Vec<RecentGame>,
    pub wishlist: Result<Option<Vec<AppId>>, SteamError>,
    pub followed: Result<Option<Vec<AppId>>, SteamError>,
}

impl SteamSignals {
    pub fn into_reading(self, now: Timestamp) -> Result<SignalsReading, SteamError> {
        Ok(SignalsReading {
            read_at: now,
            library: self.library,
            recent: self.recent,
            wishlist: self.wishlist?,
            followed: self.followed?,
            wishlist_read_at: Some(now),
            followed_read_at: Some(now),
        })
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    response: T,
}

#[derive(Deserialize)]
struct OwnedGamesResponse {
    games: Option<Vec<OwnedGame>>,
}

#[derive(Deserialize)]
struct OwnedGame {
    appid: u32,
    playtime_forever: u64,
}

#[derive(Deserialize)]
struct RecentlyPlayedResponse {
    #[serde(default)]
    games: Vec<RecentlyPlayedGame>,
}

#[derive(Deserialize)]
struct RecentlyPlayedGame {
    appid: u32,
    playtime_2weeks: u64,
    playtime_forever: u64,
}

#[derive(Deserialize)]
struct WishlistResponse {
    items: Option<Vec<WishlistItem>>,
}

#[derive(Deserialize)]
struct WishlistItem {
    appid: u32,
}

#[derive(Deserialize)]
struct FollowedResponse {
    appids: Option<Vec<u32>>,
}

pub async fn read_signals<S: Get>(
    steam: &S,
    steam_id: SteamId,
) -> Result<SteamSignals, SteamError> {
    let (library, recent, wishlist, followed) = tokio::join!(
        get_owned_games(steam, steam_id),
        get_recently_played_games(steam, steam_id),
        get_wishlist(steam, steam_id),
        get_games_followed(steam, steam_id),
    );

    Ok(SteamSignals {
        library: library?,
        recent: recent?,
        wishlist,
        followed,
    })
}

pub async fn get_owned_games<S: Get>(
    steam: &S,
    steam_id: SteamId,
) -> Result<Vec<LibraryGame>, SteamError> {
    let id = steam_id.0.to_string();
    let params = [("steamid", id.as_str()), ("include_played_free_games", "1")];
    let value = steam.get(OWNED_GAMES, &params, true).await?;
    parse_owned_games(steam_id, value)
}

pub async fn get_recently_played_games<S: Get>(
    steam: &S,
    steam_id: SteamId,
) -> Result<Vec<RecentGame>, SteamError> {
    let id = steam_id.0.to_string();
    let value = steam.get(RECENTLY_PLAYED, &[("steamid", &id)], true).await?;
    parse_recently_played_games(value)
}

pub async fn get_wishlist<S: Get>(
    steam: &S,
    steam_id: SteamId,
) -> Result<Option<Vec<AppId>>, SteamError> {
    let id = steam_id.0.to_string();
    let value = steam.get(WISHLIST, &[("steamid", &id)], false).await?;
    parse_wishlist(value)
}

pub async fn get_games_followed<S: Get>(
    steam: &S,
    steam_id: SteamId,
) -> Result<Option<Vec<AppId>>, SteamError> {
    let id = steam_id.0.to_string();
    let value = steam.get(FOLLOWED, &[("steamid", &id)], false).await?;
    parse_games_followed(value)
}

fn parse_owned_games(steam_id: SteamId, value: Value) -> Result<Vec<LibraryGame>, SteamError> {
    let games = parse::<Envelope<OwnedGamesResponse>>(OWNED_GAMES, value)?
        .response
        .games
        .unwrap_or_default();
    if games.is_empty() {
        return Err(SteamError::LibraryHidden {
            steam_id: steam_id.0,
        });
    }

    Ok(games
        .into_iter()
        .map(|game| LibraryGame {
            id: AppId(game.appid),
            total_hours: hours(game.playtime_forever),
        })
        .collect())
}

fn parse_recently_played_games(value: Value) -> Result<Vec<RecentGame>, SteamError> {
    let games = parse::<Envelope<RecentlyPlayedResponse>>(RECENTLY_PLAYED, value)?
        .response
        .games;

    Ok(games
        .into_iter()
        .map(|game| RecentGame {
            id: AppId(game.appid),
            total_hours: hours(game.playtime_forever),
            recent_hours: hours(game.playtime_2weeks),
        })
        .collect())
}

fn parse_wishlist(value: Value) -> Result<Option<Vec<AppId>>, SteamError> {
    let items = parse::<Envelope<WishlistResponse>>(WISHLIST, value)?
        .response
        .items;

    Ok(items.map(|items| items.into_iter().map(|item| AppId(item.appid)).collect()))
}

fn parse_games_followed(value: Value) -> Result<Option<Vec<AppId>>, SteamError> {
    let appids = parse::<Envelope<FollowedResponse>>(FOLLOWED, value)?
        .response
        .appids;

    Ok(appids.map(|appids| appids.into_iter().map(AppId).collect()))
}

fn hours(minutes: u64) -> f64 {
    minutes as f64 / 60.0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    const ME: SteamId = SteamId(76_561_198_000_000_000);

    struct FakeSteam {
        answers: HashMap<&'static str, Value>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            assert!(params.contains(&("steamid", "76561198000000000")));
            self.answers
                .get(endpoint)
                .cloned()
                .ok_or_else(|| SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "no answer".to_string(),
                    source: None,
                })
        }
    }

    fn steam(answers: &[(&'static str, Value)]) -> FakeSteam {
        FakeSteam {
            answers: answers.iter().cloned().collect(),
        }
    }

    fn library() -> (&'static str, Value) {
        (
            OWNED_GAMES,
            json!({ "response": { "games": [{ "appid": 10, "playtime_forever": 90 }] } }),
        )
    }

    fn recent() -> (&'static str, Value) {
        (
            RECENTLY_PLAYED,
            json!({ "response": { "games": [{ "appid": 30, "playtime_2weeks": 15, "playtime_forever": 120 }] } }),
        )
    }

    #[test]
    fn reads_the_library_in_hours() {
        let value = json!({
            "response": {
                "game_count": 2,
                "games": [
                    { "appid": 10, "playtime_forever": 90, "playtime_windows_forever": 90 },
                    { "appid": 20, "playtime_forever": 0 }
                ]
            }
        });

        let library = parse_owned_games(ME, value).unwrap();

        assert_eq!(
            library,
            [
                LibraryGame {
                    id: AppId(10),
                    total_hours: 1.5,
                },
                LibraryGame {
                    id: AppId(20),
                    total_hours: 0.0,
                },
            ]
        );
    }

    #[test]
    fn a_hidden_library_names_the_likely_causes() {
        let error = parse_owned_games(ME, json!({ "response": {} })).unwrap_err();

        assert!(matches!(error, SteamError::LibraryHidden { .. }));
        let message = error.to_string();
        assert!(message.contains("76561198000000000"));
        assert!(message.contains("public"));
        assert!(message.contains("steam_id"));
        assert!(message.contains("STEAM_API_KEY"));
    }

    #[test]
    fn an_empty_library_is_hidden_too() {
        let error = parse_owned_games(ME, json!({ "response": { "games": [] } })).unwrap_err();

        assert!(matches!(error, SteamError::LibraryHidden { .. }));
    }

    #[test]
    fn a_missing_playtime_is_a_shape_error() {
        let value = json!({ "response": { "games": [{ "appid": 10 }] } });

        let error = parse_owned_games(ME, value).unwrap_err();

        assert!(matches!(error, SteamError::Shape { .. }));
        assert!(error.to_string().starts_with(OWNED_GAMES));
    }

    #[test]
    fn no_recent_games_is_an_empty_list() {
        let recent = parse_recently_played_games(json!({ "response": { "total_count": 0 } }));

        assert_eq!(recent.unwrap(), []);
    }

    #[test]
    fn a_hidden_wishlist_or_followed_list_is_none() {
        assert_eq!(parse_wishlist(json!({ "response": {} })).unwrap(), None);
        assert_eq!(parse_games_followed(json!({ "response": {} })).unwrap(), None);
    }

    #[tokio::test]
    async fn reads_the_four_signals_in_hours() {
        let steam = steam(&[
            library(),
            recent(),
            (
                WISHLIST,
                json!({ "response": { "items": [{ "appid": 400, "priority": 1 }] } }),
            ),
            (FOLLOWED, json!({ "response": { "appids": [500, 600] } })),
        ]);

        let signals = read_signals(&steam, ME).await.unwrap();

        assert_eq!(
            signals.library,
            [LibraryGame {
                id: AppId(10),
                total_hours: 1.5,
            }]
        );
        assert_eq!(
            signals.recent,
            [RecentGame {
                id: AppId(30),
                total_hours: 2.0,
                recent_hours: 0.25,
            }]
        );
        assert_eq!(signals.wishlist.unwrap(), Some(vec![AppId(400)]));
        assert_eq!(signals.followed.unwrap(), Some(vec![AppId(500), AppId(600)]));
    }

    #[tokio::test]
    async fn a_failed_wishlist_or_followed_list_does_not_stop_the_reading() {
        let steam = steam(&[
            library(),
            recent(),
            (WISHLIST, json!({ "response": { "items": "changed" } })),
        ]);

        let signals = read_signals(&steam, ME).await.unwrap();

        assert_eq!(signals.library.len(), 1);
        assert!(matches!(signals.wishlist, Err(SteamError::Shape { .. })));
        assert!(matches!(signals.followed, Err(SteamError::SteamDown { .. })));
    }

    #[tokio::test]
    async fn a_reading_is_dated_now_even_with_hidden_lists() {
        let steam = steam(&[
            library(),
            recent(),
            (WISHLIST, json!({ "response": {} })),
            (FOLLOWED, json!({ "response": { "appids": [500] } })),
        ]);
        let now = Timestamp(1_793_638_800_000);

        let reading = read_signals(&steam, ME)
            .await
            .unwrap()
            .into_reading(now)
            .unwrap();

        assert_eq!(reading.read_at, now);
        assert_eq!(reading.wishlist, None);
        assert_eq!(reading.wishlist_read_at, Some(now));
        assert_eq!(reading.followed, Some(vec![AppId(500)]));
        assert_eq!(reading.followed_read_at, Some(now));
    }

    #[tokio::test]
    async fn a_failed_library_or_recent_list_stops_the_reading() {
        let without_recent = read_signals(&steam(&[library()]), ME).await;
        let without_library = read_signals(&steam(&[recent()]), ME).await;

        assert!(matches!(without_recent, Err(SteamError::SteamDown { .. })));
        assert!(matches!(without_library, Err(SteamError::SteamDown { .. })));
    }
}
