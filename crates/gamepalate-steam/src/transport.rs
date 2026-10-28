use std::{
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use reqwest::{
    Client, Method, Request, Response, StatusCode, Url,
    header::{ACCEPT, HeaderMap, HeaderValue},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::time::{Instant, timeout_at};

const BASE_URL: &str = "https://api.steampowered.com/";
const USER_AGENT: &str = "gamepalate/0.1 (personal tool)";
const TIMEOUT: Duration = Duration::from_secs(10);
const INTERVAL: Duration = Duration::from_millis(500);
const MAX_INTERVAL: Duration = Duration::from_secs(2);
const SLOW_DOWN_STEP: Duration = Duration::from_millis(250);
const SPEED_UP_STEP: Duration = Duration::from_millis(25);
const FIRST_BACKOFF: Duration = Duration::from_secs(2);
const MAX_ATTEMPTS: u32 = 7;
const TRANSIENT_WAITS: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(45),
    Duration::from_secs(90),
];

#[derive(Debug, thiserror::Error)]
pub enum SteamError {
    #[error(
        "{endpoint} needs a Steam Web API key: set STEAM_API_KEY, you can get one at https://steamcommunity.com/dev/apikey"
    )]
    MissingKey { endpoint: String },

    #[error("Steam rejected the API key on {endpoint} (HTTP {status}): check STEAM_API_KEY")]
    KeyRejected {
        endpoint: String,
        status: StatusCode,
    },

    #[error("Steam is not responding on {endpoint}: {reason}")]
    SteamDown {
        endpoint: String,
        reason: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    #[error("{endpoint} answered HTTP {status}{}", if *status == StatusCode::NOT_FOUND { ", the endpoint may have changed" } else { "" })]
    Status {
        endpoint: String,
        status: StatusCode,
    },

    #[error("{endpoint} answered with an unexpected shape: {reason}")]
    Shape { endpoint: String, reason: String },

    #[error("could not set up the HTTP client")]
    Client(#[source] reqwest::Error),
}

pub(crate) fn parse<T: DeserializeOwned>(endpoint: &str, value: Value) -> Result<T, SteamError> {
    serde_json::from_value(value).map_err(|error| SteamError::Shape {
        endpoint: endpoint.to_string(),
        reason: error.to_string(),
    })
}

pub trait Fetch {
    fn fetch(&self, request: Request) -> impl Future<Output = reqwest::Result<Response>> + Send;
}

impl Fetch for Client {
    fn fetch(&self, request: Request) -> impl Future<Output = reqwest::Result<Response>> + Send {
        self.execute(request)
    }
}

pub struct Transport<F = Client> {
    fetch: F,
    api_key: Option<String>,
    pace: Mutex<Pace>,
}

struct Pace {
    next_slot: Instant,
    interval: Duration,
}

enum Attempt {
    Done(Value),
    RateLimited,
    Transient(String),
}

impl Transport {
    pub fn new(api_key: Option<String>) -> Result<Transport, SteamError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .build()
            .map_err(|error| SteamError::Client(error.without_url()))?;

        Ok(Transport::with_fetch(client, api_key))
    }
}

impl<F: Fetch> Transport<F> {
    pub fn with_fetch(fetch: F, api_key: Option<String>) -> Transport<F> {
        Transport {
            fetch,
            api_key,
            pace: Mutex::new(Pace {
                next_slot: Instant::now(),
                interval: INTERVAL,
            }),
        }
    }

    pub async fn get(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
        needs_key: bool,
    ) -> Result<Value, SteamError> {
        let key = if needs_key {
            Some(
                self.api_key
                    .as_deref()
                    .ok_or_else(|| SteamError::MissingKey {
                        endpoint: endpoint.to_string(),
                    })?,
            )
        } else {
            None
        };

        let mut backoff = FIRST_BACKOFF;
        let mut waits = TRANSIENT_WAITS.into_iter();
        for attempt in 1..=MAX_ATTEMPTS {
            match self.send_once(endpoint, params, key).await? {
                Attempt::Done(value) => {
                    self.speed_up();
                    return Ok(value);
                }
                Attempt::RateLimited if attempt < MAX_ATTEMPTS => {
                    self.slow_down(backoff);
                    backoff *= 2;
                }
                Attempt::RateLimited => {}
                Attempt::Transient(reason) => match waits.next() {
                    Some(wait) if attempt < MAX_ATTEMPTS => {
                        self.pause(wait);
                        log::warn!("{endpoint}: {reason}, trying again in {wait:?}");
                    }
                    _ => {
                        return Err(SteamError::SteamDown {
                            endpoint: endpoint.to_string(),
                            reason,
                            source: None,
                        });
                    }
                },
            }
        }

        Err(SteamError::SteamDown {
            endpoint: endpoint.to_string(),
            reason: format!("still rate limited after {MAX_ATTEMPTS} attempts"),
            source: None,
        })
    }

    async fn send_once(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
        key: Option<&str>,
    ) -> Result<Attempt, SteamError> {
        let url = Url::parse_with_params(
            &format!("{BASE_URL}{endpoint}/"),
            params.iter().copied().chain(key.map(|key| ("key", key))),
        )
        .expect("the endpoint makes a valid URL");

        self.wait_for_slot().await;
        let deadline = Instant::now() + TIMEOUT;
        let Ok(response) =
            timeout_at(deadline, self.fetch.fetch(Request::new(Method::GET, url))).await
        else {
            return Ok(Attempt::Transient("timed out".to_string()));
        };
        let response = response.map_err(|error| steam_down(endpoint, error))?;

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(Attempt::RateLimited);
        }
        if key.is_some() && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(SteamError::KeyRejected {
                endpoint: endpoint.to_string(),
                status,
            });
        }
        if matches!(
            status,
            StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
        ) {
            return Ok(Attempt::Transient(format!("HTTP {status}")));
        }
        if status.is_server_error() {
            return Err(SteamError::SteamDown {
                endpoint: endpoint.to_string(),
                reason: format!("HTTP {status}"),
                source: None,
            });
        }
        if !status.is_success() {
            return Err(SteamError::Status {
                endpoint: endpoint.to_string(),
                status,
            });
        }

        let Ok(value) = timeout_at(deadline, response.json()).await else {
            return Ok(Attempt::Transient("timed out".to_string()));
        };
        let value = value.map_err(|error| steam_down(endpoint, error))?;
        Ok(Attempt::Done(value))
    }

    async fn wait_for_slot(&self) {
        let slot = {
            let mut pace = self.lock_pace();
            let slot = pace.next_slot.max(Instant::now());
            pace.next_slot = slot + pace.interval;
            slot
        };

        tokio::time::sleep_until(slot).await;
    }

    fn pause(&self, wait: Duration) {
        let mut pace = self.lock_pace();
        pace.next_slot = pace.next_slot.max(Instant::now() + wait);
    }

    fn slow_down(&self, wait: Duration) {
        let mut pace = self.lock_pace();
        pace.interval = slower(pace.interval);
        pace.next_slot = pace.next_slot.max(Instant::now() + wait);
        log::warn!(
            "rate limited: waiting {wait:?}, interval now {:?}",
            pace.interval
        );
    }

    fn speed_up(&self) {
        let mut pace = self.lock_pace();
        pace.interval = faster(pace.interval);
    }

    fn lock_pace(&self) -> MutexGuard<'_, Pace> {
        self.pace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn slower(interval: Duration) -> Duration {
    (interval + SLOW_DOWN_STEP).min(MAX_INTERVAL)
}

fn faster(interval: Duration) -> Duration {
    interval.saturating_sub(SPEED_UP_STEP).max(INTERVAL)
}

fn steam_down(endpoint: &str, error: reqwest::Error) -> SteamError {
    let reason = if error.is_decode() {
        "the answer is not JSON"
    } else {
        "no answer"
    };

    SteamError::SteamDown {
        endpoint: endpoint.to_string(),
        reason: reason.to_string(),
        source: Some(error.without_url()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    const TAG_LIST: &str = "IStoreService/GetTagList/v1";
    const APP_LIST: &str = "IStoreService/GetAppList/v1";

    enum Reply {
        Answer { status: u16, body: &'static str },
        Hang,
    }

    fn ok() -> Reply {
        answer(200, r#"{"response":{}}"#)
    }

    fn answer(status: u16, body: &'static str) -> Reply {
        Reply::Answer { status, body }
    }

    fn rate_limited() -> Reply {
        answer(429, "")
    }

    struct FakeSteam {
        replies: Mutex<VecDeque<Reply>>,
        requests: Mutex<Vec<(Instant, Url)>>,
    }

    impl Fetch for FakeSteam {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            self.requests
                .lock()
                .unwrap()
                .push((Instant::now(), request.url().clone()));
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("one reply for every request");

            async move {
                match reply {
                    Reply::Hang => std::future::pending().await,
                    Reply::Answer { status, body } => Ok(http::Response::builder()
                        .status(status)
                        .body(body)
                        .unwrap()
                        .into()),
                }
            }
        }
    }

    fn fake_transport(replies: Vec<Reply>, api_key: Option<&str>) -> Transport<FakeSteam> {
        let fake = FakeSteam {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        };
        Transport::with_fetch(fake, api_key.map(str::to_string))
    }

    fn start_offsets_ms(transport: &Transport<FakeSteam>, start: Instant) -> Vec<u128> {
        transport
            .fetch
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(sent_at, _)| (*sent_at - start).as_millis())
            .collect()
    }

    async fn get_tags(transport: &Transport<FakeSteam>) -> Result<Value, SteamError> {
        transport
            .get(TAG_LIST, &[("language", "english")], false)
            .await
    }

    fn steam_down_reason(result: Result<Value, SteamError>) -> String {
        match result {
            Err(SteamError::SteamDown { reason, .. }) => reason,
            other => panic!("expected SteamDown, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_requests_start_one_interval_apart() {
        let start = Instant::now();
        let transport = fake_transport(vec![ok(), ok(), ok()], None);

        let (first, second, third) = tokio::join!(
            get_tags(&transport),
            get_tags(&transport),
            get_tags(&transport)
        );

        assert!(first.is_ok() && second.is_ok() && third.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 500, 1000]);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limited_request_is_retried_with_doubling_waits() {
        let start = Instant::now();
        let transport = fake_transport(vec![rate_limited(), rate_limited(), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 2000, 6000]);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_slows_down_later_requests() {
        let start = Instant::now();
        let transport = fake_transport(vec![rate_limited(), ok(), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 2000, 2750]);
    }

    #[tokio::test(start_paused = true)]
    async fn every_good_answer_shortens_the_interval() {
        let start = Instant::now();
        let mut replies = vec![rate_limited()];
        replies.extend((0..12).map(|_| ok()));
        let transport = fake_transport(replies, None);

        for _ in 0..12 {
            assert!(get_tags(&transport).await.is_ok());
        }

        assert_eq!(
            start_offsets_ms(&transport, start),
            [
                0, 2000, 2750, 3475, 4175, 4850, 5500, 6125, 6725, 7300, 7850, 8375, 8875
            ]
        );
    }

    #[test]
    fn the_interval_stays_between_half_a_second_and_two_seconds() {
        assert_eq!(slower(Duration::from_millis(1900)), MAX_INTERVAL);
        assert_eq!(faster(Duration::from_millis(520)), INTERVAL);
        assert_eq!(faster(INTERVAL), INTERVAL);
    }

    #[tokio::test(start_paused = true)]
    async fn seven_rate_limits_give_up() {
        let start = Instant::now();
        let mut replies: Vec<Reply> = (0..7).map(|_| rate_limited()).collect();
        replies.push(ok());
        let transport = fake_transport(replies, None);

        let reason = steam_down_reason(get_tags(&transport).await);
        assert!(get_tags(&transport).await.is_ok());

        assert_eq!(reason, "still rate limited after 7 attempts");
        assert_eq!(
            start_offsets_ms(&transport, start),
            [0, 2000, 6000, 14_000, 30_000, 62_000, 126_000, 128_000]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_request_is_retried_after_five_seconds() {
        let start = Instant::now();
        let transport = fake_transport(vec![Reply::Hang, ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 15_000]);
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_five_times_gives_up() {
        let start = Instant::now();
        let transport = fake_transport((0..5).map(|_| Reply::Hang).collect(), None);

        assert_eq!(steam_down_reason(get_tags(&transport).await), "timed out");
        assert_eq!(
            start_offsets_ms(&transport, start),
            [0, 15_000, 40_000, 95_000, 195_000]
        );
        assert_eq!(Instant::now() - start, Duration::from_secs(205));
    }

    #[tokio::test(start_paused = true)]
    async fn answer_that_is_not_json_means_steam_is_down() {
        let transport = fake_transport(vec![answer(200, "<html></html>")], None);

        assert_eq!(
            steam_down_reason(get_tags(&transport).await),
            "the answer is not JSON"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn bad_gateway_is_retried_with_growing_waits() {
        let start = Instant::now();
        let transport = fake_transport(vec![answer(502, ""), answer(504, ""), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 5000, 20_000]);
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_five_times_means_steam_is_down() {
        let start = Instant::now();
        let transport = fake_transport((0..5).map(|_| answer(503, "")).collect(), None);

        assert_eq!(
            steam_down_reason(get_tags(&transport).await),
            "HTTP 503 Service Unavailable"
        );
        assert_eq!(
            start_offsets_ms(&transport, start),
            [0, 5000, 20_000, 65_000, 155_000]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn transient_errors_do_not_slow_down_later_requests() {
        let start = Instant::now();
        let transport = fake_transport(vec![answer(502, ""), ok(), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 5000, 5500]);
    }

    #[tokio::test(start_paused = true)]
    async fn internal_server_error_means_steam_is_down_at_once() {
        let transport = fake_transport(vec![answer(500, "")], None);

        assert_eq!(
            steam_down_reason(get_tags(&transport).await),
            "HTTP 500 Internal Server Error"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn not_found_is_a_status_error() {
        let transport = fake_transport(vec![answer(404, "")], None);

        let result = get_tags(&transport).await;
        assert!(matches!(
            result,
            Err(SteamError::Status { status, .. }) if status == StatusCode::NOT_FOUND
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn key_goes_only_where_it_is_needed() {
        let transport = fake_transport(vec![ok(), ok()], Some("secret"));

        assert!(get_tags(&transport).await.is_ok());
        assert!(transport.get(APP_LIST, &[], true).await.is_ok());

        let requests = transport.fetch.requests.lock().unwrap();
        let keys: Vec<Option<String>> = requests
            .iter()
            .map(|(_, url)| {
                url.query_pairs()
                    .find(|(name, _)| name == "key")
                    .map(|(_, value)| value.into_owned())
            })
            .collect();
        assert_eq!(keys, [None, Some("secret".to_string())]);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_key_sends_nothing() {
        let transport = fake_transport(vec![], None);

        let result = transport.get(APP_LIST, &[], true).await;

        assert!(matches!(result, Err(SteamError::MissingKey { .. })));
        assert!(transport.fetch.requests.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn forbidden_with_a_key_means_the_key_is_rejected() {
        let transport = fake_transport(vec![answer(403, "")], Some("secret"));

        let error = transport.get(APP_LIST, &[], true).await.unwrap_err();

        assert!(matches!(error, SteamError::KeyRejected { .. }));
        assert!(!format!("{error:?}").contains("secret"));
    }
}
