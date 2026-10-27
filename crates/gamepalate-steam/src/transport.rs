use std::{
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use reqwest::{
    Client, Method, Request, Response, StatusCode, Url,
    header::{ACCEPT, HeaderMap, HeaderValue, RETRY_AFTER},
};
use serde_json::Value;
use tokio::time::{Instant, timeout_at};

const BASE_URL: &str = "https://api.steampowered.com/";
const USER_AGENT: &str = "gamepalate/0.1 (personal tool)";
const TIMEOUT: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_millis(500);
const MAX_INTERVAL: Duration = Duration::from_secs(4);
const FIRST_BACKOFF: Duration = Duration::from_secs(2);
const MAX_ATTEMPTS: u32 = 7;

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

    #[error("could not set up the HTTP client")]
    Client(#[source] reqwest::Error),
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
    RateLimited(Option<Duration>),
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
        for attempt in 1..=MAX_ATTEMPTS {
            match self.send_once(endpoint, params, key).await? {
                Attempt::Done(value) => return Ok(value),
                Attempt::RateLimited(retry_after) if attempt < MAX_ATTEMPTS => {
                    self.slow_down(retry_after.map_or(backoff, |wait| wait.max(backoff)));
                    backoff *= 2;
                }
                Attempt::RateLimited(_) => {}
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
        let response = timeout_at(deadline, self.fetch.fetch(Request::new(Method::GET, url)))
            .await
            .map_err(|_| timed_out(endpoint))?
            .map_err(|error| steam_down(endpoint, error))?;

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(Attempt::RateLimited(retry_after(&response)));
        }
        if key.is_some() && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(SteamError::KeyRejected {
                endpoint: endpoint.to_string(),
                status,
            });
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

        let value = timeout_at(deadline, response.json())
            .await
            .map_err(|_| timed_out(endpoint))?
            .map_err(|error| steam_down(endpoint, error))?;
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

    fn slow_down(&self, wait: Duration) {
        let mut pace = self.lock_pace();
        pace.interval = (pace.interval * 2).min(MAX_INTERVAL);
        pace.next_slot = pace.next_slot.max(Instant::now() + wait);
    }

    fn lock_pace(&self) -> MutexGuard<'_, Pace> {
        self.pace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn retry_after(response: &Response) -> Option<Duration> {
    let seconds = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    seconds.trim().parse().ok().map(Duration::from_secs)
}

fn timed_out(endpoint: &str) -> SteamError {
    SteamError::SteamDown {
        endpoint: endpoint.to_string(),
        reason: "timed out".to_string(),
        source: None,
    }
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
        Answer {
            status: u16,
            retry_after: Option<&'static str>,
            body: &'static str,
        },
        Hang,
    }

    fn ok() -> Reply {
        answer(200, r#"{"response":{}}"#)
    }

    fn answer(status: u16, body: &'static str) -> Reply {
        Reply::Answer {
            status,
            retry_after: None,
            body,
        }
    }

    fn rate_limited(retry_after: Option<&'static str>) -> Reply {
        Reply::Answer {
            status: 429,
            retry_after,
            body: "",
        }
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
                    Reply::Answer {
                        status,
                        retry_after,
                        body,
                    } => {
                        let mut response = http::Response::builder().status(status);
                        if let Some(retry_after) = retry_after {
                            response = response.header(RETRY_AFTER, retry_after);
                        }
                        Ok(response.body(body).unwrap().into())
                    }
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
        let transport = fake_transport(vec![rate_limited(None), rate_limited(None), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 2000, 6000]);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_after_longer_than_the_backoff_wins() {
        let start = Instant::now();
        let transport = fake_transport(vec![rate_limited(Some("30")), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 30_000]);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_slows_down_later_requests() {
        let start = Instant::now();
        let transport = fake_transport(vec![rate_limited(None), ok(), ok()], None);

        assert!(get_tags(&transport).await.is_ok());
        assert!(get_tags(&transport).await.is_ok());
        assert_eq!(start_offsets_ms(&transport, start), [0, 2000, 3000]);
    }

    #[tokio::test(start_paused = true)]
    async fn seven_rate_limits_give_up_and_cap_the_interval_at_four_seconds() {
        let start = Instant::now();
        let mut replies: Vec<Reply> = (0..7).map(|_| rate_limited(None)).collect();
        replies.push(ok());
        let transport = fake_transport(replies, None);

        let reason = steam_down_reason(get_tags(&transport).await);
        assert!(get_tags(&transport).await.is_ok());

        assert_eq!(reason, "still rate limited after 7 attempts");
        assert_eq!(
            start_offsets_ms(&transport, start),
            [0, 2000, 6000, 14_000, 30_000, 62_000, 126_000, 130_000]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_request_times_out_after_a_minute() {
        let start = Instant::now();
        let transport = fake_transport(vec![Reply::Hang], None);

        assert_eq!(steam_down_reason(get_tags(&transport).await), "timed out");
        assert_eq!(Instant::now() - start, TIMEOUT);
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
    async fn server_error_means_steam_is_down() {
        let transport = fake_transport(vec![answer(503, "")], None);

        assert_eq!(
            steam_down_reason(get_tags(&transport).await),
            "HTTP 503 Service Unavailable"
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
