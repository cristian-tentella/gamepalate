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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionalList {
    Wishlist,
    Followed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FailedList {
    pub list: OptionalList,
    pub saved_from: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalsRead {
    pub reading: SignalsReading,
    pub saved_from: Option<Timestamp>,
    pub failed: Vec<FailedList>,
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
    saved: Option<SignalsReading>,
    now: Timestamp,
) -> Result<SignalsRead, SteamError> {
    let (library, recent, wishlist, followed) = tokio::join!(
        get_owned_games(steam, steam_id),
        get_recently_played_games(steam, steam_id),
        get_wishlist(steam, steam_id),
        get_games_followed(steam, steam_id),
    );

    let (library, recent) = match both(library, recent) {
        Ok(lists) => lists,
        Err(error) => {
            return match saved {
                Some(saved) if is_down(&error) => Ok(SignalsRead {
                    saved_from: Some(saved.read_at),
                    reading: saved,
                    failed: Vec::new(),
                }),
                _ => Err(error),
            };
        }
    };

    let mut failed = Vec::new();
    let (wishlist, wishlist_read_at) = settle(
        OptionalList::Wishlist,
        wishlist,
        saved.as_ref(),
        now,
        &mut failed,
    );
    let (followed, followed_read_at) = settle(
        OptionalList::Followed,
        followed,
        saved.as_ref(),
        now,
        &mut failed,
    );

    Ok(SignalsRead {
        reading: SignalsReading {
            read_at: now,
            library,
            recent,
            wishlist,
            followed,
            wishlist_read_at,
            followed_read_at,
        },
        saved_from: None,
        failed,
    })
}

fn both<A, B>(a: Result<A, SteamError>, b: Result<B, SteamError>) -> Result<(A, B), SteamError> {
    match (a, b) {
        (Ok(a), Ok(b)) => Ok((a, b)),
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
        (Err(a), Err(b)) => Err(if is_down(&a) { b } else { a }),
    }
}

fn is_down(error: &SteamError) -> bool {
    matches!(error, SteamError::SteamDown { .. })
}

fn settle(
    list: OptionalList,
    read: Result<Option<Vec<AppId>>, SteamError>,
    saved: Option<&SignalsReading>,
    now: Timestamp,
    failed: &mut Vec<FailedList>,
) -> (Option<Vec<AppId>>, Option<Timestamp>) {
    if let Ok(ids) = read {
        return (ids, Some(now));
    }
    let Some(saved) = saved else {
        failed.push(FailedList {
            list,
            saved_from: None,
        });
        return (None, None);
    };

    let (ids, read_at) = match list {
        OptionalList::Wishlist => (&saved.wishlist, saved.wishlist_read_at),
        OptionalList::Followed => (&saved.followed, saved.followed_read_at),
    };
    let read_at = read_at.unwrap_or(saved.read_at);
    failed.push(FailedList {
        list,
        saved_from: ids.as_ref().map(|_| read_at),
    });
    (ids.clone(), Some(read_at))
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
    let value = steam
        .get(RECENTLY_PLAYED, &[("steamid", &id)], true)
        .await?;
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
        key_rejected: bool,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            needs_key: bool,
        ) -> Result<Value, SteamError> {
            assert!(params.contains(&("steamid", "76561198000000000")));
            if needs_key && self.key_rejected {
                return Err(SteamError::KeyRejected {
                    endpoint: endpoint.to_string(),
                    status: reqwest::StatusCode::FORBIDDEN,
                });
            }
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
            key_rejected: false,
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
        assert_eq!(
            parse_games_followed(json!({ "response": {} })).unwrap(),
            None
        );
    }

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const BEFORE: Timestamp = Timestamp(1_793_000_000_000);
    const LONG_BEFORE: Timestamp = Timestamp(1_792_000_000_000);

    fn wishlist() -> (&'static str, Value) {
        (
            WISHLIST,
            json!({ "response": { "items": [{ "appid": 400, "priority": 1 }] } }),
        )
    }

    fn followed() -> (&'static str, Value) {
        (FOLLOWED, json!({ "response": { "appids": [500, 600] } }))
    }

    fn saved() -> SignalsReading {
        SignalsReading {
            read_at: BEFORE,
            library: vec![LibraryGame {
                id: AppId(1),
                total_hours: 10.0,
            }],
            recent: vec![],
            wishlist: Some(vec![AppId(4)]),
            followed: Some(vec![AppId(5)]),
            wishlist_read_at: Some(LONG_BEFORE),
            followed_read_at: None,
        }
    }

    #[tokio::test]
    async fn reads_the_four_signals_in_hours() {
        let steam = steam(&[library(), recent(), wishlist(), followed()]);

        let read = read_signals(&steam, ME, Some(saved()), NOW).await.unwrap();

        assert_eq!(
            read,
            SignalsRead {
                reading: SignalsReading {
                    read_at: NOW,
                    library: vec![LibraryGame {
                        id: AppId(10),
                        total_hours: 1.5,
                    }],
                    recent: vec![RecentGame {
                        id: AppId(30),
                        total_hours: 2.0,
                        recent_hours: 0.25,
                    }],
                    wishlist: Some(vec![AppId(400)]),
                    followed: Some(vec![AppId(500), AppId(600)]),
                    wishlist_read_at: Some(NOW),
                    followed_read_at: Some(NOW),
                },
                saved_from: None,
                failed: vec![],
            }
        );
    }

    #[tokio::test]
    async fn a_hidden_list_is_read_now() {
        let steam = steam(&[
            library(),
            recent(),
            (WISHLIST, json!({ "response": {} })),
            followed(),
        ]);

        let read = read_signals(&steam, ME, None, NOW).await.unwrap();

        assert_eq!(read.reading.wishlist, None);
        assert_eq!(read.reading.wishlist_read_at, Some(NOW));
        assert!(read.failed.is_empty());
    }

    #[tokio::test]
    async fn a_failed_list_uses_the_saved_one_with_its_date() {
        let steam = steam(&[
            library(),
            recent(),
            (WISHLIST, json!({ "response": { "items": "changed" } })),
        ]);

        let read = read_signals(&steam, ME, Some(saved()), NOW).await.unwrap();

        assert_eq!(read.reading.read_at, NOW);
        assert_eq!(read.reading.library[0].id, AppId(10));
        assert_eq!(read.reading.wishlist, Some(vec![AppId(4)]));
        assert_eq!(read.reading.wishlist_read_at, Some(LONG_BEFORE));
        assert_eq!(read.reading.followed, Some(vec![AppId(5)]));
        assert_eq!(read.reading.followed_read_at, Some(BEFORE));
        assert_eq!(
            read.failed,
            [
                FailedList {
                    list: OptionalList::Wishlist,
                    saved_from: Some(LONG_BEFORE),
                },
                FailedList {
                    list: OptionalList::Followed,
                    saved_from: Some(BEFORE),
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_list_without_a_saved_one_is_left_out() {
        let mut hidden = saved();
        hidden.followed = None;
        let steam = steam(&[library(), recent(), wishlist()]);

        let without_reading = read_signals(&steam, ME, None, NOW).await.unwrap();
        let without_list = read_signals(&steam, ME, Some(hidden), NOW).await.unwrap();

        assert_eq!(without_reading.reading.followed, None);
        assert_eq!(without_reading.reading.followed_read_at, None);
        assert_eq!(without_list.reading.followed, None);
        assert_eq!(without_list.reading.followed_read_at, Some(BEFORE));
        for read in [without_reading, without_list] {
            assert_eq!(
                read.failed,
                [FailedList {
                    list: OptionalList::Followed,
                    saved_from: None,
                }]
            );
        }
    }

    #[tokio::test]
    async fn steam_down_uses_the_whole_saved_reading() {
        let steam = steam(&[library(), wishlist(), followed()]);

        let read = read_signals(&steam, ME, Some(saved()), NOW).await.unwrap();

        assert_eq!(
            read,
            SignalsRead {
                reading: saved(),
                saved_from: Some(BEFORE),
                failed: vec![],
            }
        );
    }

    #[tokio::test]
    async fn steam_down_without_a_saved_reading_is_an_error() {
        let result = read_signals(&steam(&[recent()]), ME, None, NOW).await;

        assert!(matches!(result, Err(SteamError::SteamDown { .. })));
    }

    #[tokio::test]
    async fn a_rejected_key_stops_even_with_a_saved_reading() {
        let mut steam = steam(&[library(), recent(), wishlist(), followed()]);
        steam.key_rejected = true;

        let result = read_signals(&steam, ME, Some(saved()), NOW).await;

        assert!(matches!(result, Err(SteamError::KeyRejected { .. })));
    }

    #[tokio::test]
    async fn a_hidden_library_stops_even_with_a_saved_reading() {
        let steam = steam(&[(OWNED_GAMES, json!({ "response": {} }))]);

        let result = read_signals(&steam, ME, Some(saved()), NOW).await;

        assert!(matches!(result, Err(SteamError::LibraryHidden { .. })));
    }
}
