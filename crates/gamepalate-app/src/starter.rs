use std::{collections::HashSet, time::Duration};

use gamepalate_core::{
    ids::SteamId,
    starter::{
        STARTER_CATALOG_MAX_BYTES, StarterCatalog, StarterCatalogError, parse_starter_catalog,
    },
};
use gamepalate_steam::{
    signals::get_owned_games,
    transport::{Fetch, Get},
};
use reqwest::{Method, Request, Url};
use tokio::time::timeout;

use crate::{log::failed, new_version::release_asset_url};

const FILE: &str = "starter-catalog.txt";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
enum StarterError {
    #[error("{url} is not a valid address")]
    Url { url: String },

    #[error("could not reach {url}")]
    Network {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{url} answered HTTP {status}")]
    Status {
        url: String,
        status: reqwest::StatusCode,
    },

    #[error("{url} did not answer in {}s", TIMEOUT.as_secs())]
    TimedOut { url: String },

    #[error("{url} has no appids")]
    Empty { url: String },

    #[error(transparent)]
    File(#[from] StarterCatalogError),
}

/// The apps a first download asks before the others: the Library, then the
/// popular apps of the starter catalog of the release, with its tag
/// frequencies. Without the starter catalog there is no first part.
pub(crate) async fn first_part<S: Get, F: Fetch>(
    steam: &S,
    fetch: &F,
    releases_url: &str,
    steam_id: Option<SteamId>,
) -> Option<StarterCatalog> {
    let starter = starter_catalog(fetch, releases_url)
        .await
        .inspect_err(failed(
            "could not read the starter catalog, downloading the catalog at once",
        ))
        .ok()?;
    let library = match steam_id {
        Some(steam_id) => get_owned_games(steam, steam_id)
            .await
            .inspect_err(failed("could not read the Library for the first part"))
            .map(|games| games.into_iter().map(|game| game.id).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let mut seen = HashSet::new();
    Some(StarterCatalog {
        apps: library
            .into_iter()
            .chain(starter.apps)
            .filter(|app| seen.insert(*app))
            .collect(),
        tags: starter.tags,
    })
}

async fn starter_catalog<F: Fetch>(
    fetch: &F,
    releases_url: &str,
) -> Result<StarterCatalog, StarterError> {
    let url = release_asset_url(releases_url, FILE);
    let bytes = timeout(TIMEOUT, download(fetch, &url))
        .await
        .map_err(|_| StarterError::TimedOut { url: url.clone() })??;

    let starter = parse_starter_catalog(&bytes)?;
    if starter.apps.is_empty() {
        return Err(StarterError::Empty { url });
    }
    if let Some(comment) = String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .filter(|line| line.starts_with('#'))
    {
        tracing::info!("{comment}");
    }
    Ok(starter)
}

async fn download<F: Fetch>(fetch: &F, url: &str) -> Result<Vec<u8>, StarterError> {
    let parsed = Url::parse(url).map_err(|_| StarterError::Url {
        url: url.to_string(),
    })?;
    let network = |source| StarterError::Network {
        url: url.to_string(),
        source,
    };

    let response = fetch
        .fetch(Request::new(Method::GET, parsed))
        .await
        .map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(StarterError::Status {
            url: url.to_string(),
            status,
        });
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > STARTER_CATALOG_MAX_BYTES {
            return Err(StarterCatalogError::TooLarge { bytes: bytes.len() }.into());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{future, sync::Mutex};

    use gamepalate_core::{
        catalog::TagFrequencies,
        ids::{AppId, TagId},
    };
    use gamepalate_steam::transport::SteamError;
    use serde_json::{Value, json};

    use super::*;

    const RELEASES: &str = "https://example.test/dl";
    const STEAM_ID: SteamId = SteamId(76_561_198_000_000_000);

    enum Answer {
        File(u16, Vec<u8>),
        Never,
    }

    struct FakeGithub {
        answer: Answer,
        asked: Mutex<Vec<String>>,
    }

    impl FakeGithub {
        fn new(answer: Answer) -> FakeGithub {
            FakeGithub {
                answer,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn file(text: &str) -> FakeGithub {
            FakeGithub::new(Answer::File(200, text.as_bytes().to_vec()))
        }
    }

    impl Fetch for FakeGithub {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<reqwest::Response>> + Send {
            self.asked.lock().unwrap().push(request.url().to_string());
            let response = match &self.answer {
                Answer::File(status, body) => Some(
                    http::Response::builder()
                        .status(*status)
                        .body(body.clone())
                        .unwrap(),
                ),
                Answer::Never => None,
            };
            async move {
                match response {
                    Some(response) => Ok(response.into()),
                    None => future::pending().await,
                }
            }
        }
    }

    struct FakeSteam {
        owned: Option<Vec<u32>>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            assert_eq!(endpoint, "IPlayerService/GetOwnedGames/v1");
            match &self.owned {
                Some(owned) => Ok(json!({ "response": { "games": owned
                    .iter()
                    .map(|appid| json!({ "appid": appid, "playtime_forever": 60 }))
                    .collect::<Vec<_>>() } })),
                None => Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                }),
            }
        }
    }

    fn library(owned: &[u32]) -> FakeSteam {
        FakeSteam {
            owned: Some(owned.to_vec()),
        }
    }

    fn ids(starter: Option<StarterCatalog>) -> Option<Vec<u32>> {
        starter.map(|starter| starter.apps.into_iter().map(|app| app.0).collect())
    }

    async fn first(steam: &FakeSteam, github: &FakeGithub) -> Option<Vec<u32>> {
        ids(first_part(steam, github, RELEASES, Some(STEAM_ID)).await)
    }

    #[tokio::test]
    async fn the_library_comes_first_then_the_popular_apps_without_doubles() {
        let github = FakeGithub::file("# gamepalate starter catalog\n[apps]\n730\n440\n570\n");

        let first = first(&library(&[440, 10]), &github).await;

        assert_eq!(first, Some(vec![440, 10, 730, 570]));
        assert_eq!(
            *github.asked.lock().unwrap(),
            ["https://example.test/dl/starter-catalog.txt"]
        );
    }

    #[tokio::test]
    async fn the_tags_of_the_file_come_with_the_first_part() {
        let github = FakeGithub::file("[apps]\n730\n[tags]\ngames 120\n19 48\n492 30\n");

        let starter = first_part(&library(&[10]), &github, RELEASES, Some(STEAM_ID))
            .await
            .unwrap();

        assert_eq!(starter.apps, [AppId(10), AppId(730)]);
        assert_eq!(
            starter.tags,
            Some(TagFrequencies {
                games: 120,
                tags: [(TagId(19), 48), (TagId(492), 30)].into(),
            })
        );
    }

    #[tokio::test]
    async fn a_file_without_tags_gives_a_first_part_without_tags() {
        let github = FakeGithub::file("[apps]\n730\n");

        let starter = first_part(&library(&[]), &github, RELEASES, Some(STEAM_ID))
            .await
            .unwrap();

        assert_eq!(starter.apps, [AppId(730)]);
        assert_eq!(starter.tags, None);
    }

    #[tokio::test]
    async fn without_the_library_the_first_part_is_the_file() {
        let github = FakeGithub::file("[apps]\n730\n440\n");

        assert_eq!(
            first(&FakeSteam { owned: None }, &github).await,
            Some(vec![730, 440])
        );
        assert_eq!(
            ids(first_part(&library(&[10]), &github, RELEASES, None).await),
            Some(vec![730, 440])
        );
    }

    #[tokio::test]
    async fn a_missing_file_gives_no_first_part() {
        let github = FakeGithub::new(Answer::File(404, b"Not Found".to_vec()));

        assert_eq!(first(&library(&[10]), &github).await, None);
    }

    #[tokio::test]
    async fn a_discarded_file_gives_no_first_part() {
        for text in [
            "[apps]\n730\nhello\n",
            "730\n440\n",
            "[apps]\n730\n[tags]\ngames 10\n19 3\n19 4\n",
            "[apps]\n730\n[tags]\n19 3\n",
            "[apps]\n[tags]\ngames 10\n",
            "# only a comment\n",
            "",
        ] {
            let github = FakeGithub::file(text);

            assert_eq!(first(&library(&[10]), &github).await, None, "{text:?}");
        }
    }

    #[tokio::test]
    async fn a_file_too_large_gives_no_first_part() {
        let text = format!("[apps]\n{}", "730\n".repeat(300_000));

        assert_eq!(first(&library(&[10]), &FakeGithub::file(&text)).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_file_that_never_comes_gives_no_first_part_after_five_seconds() {
        let github = FakeGithub::new(Answer::Never);
        let started = tokio::time::Instant::now();

        let error = starter_catalog(&github, RELEASES).await.unwrap_err();

        assert!(matches!(error, StarterError::TimedOut { .. }));
        assert_eq!(started.elapsed(), TIMEOUT);
        assert_eq!(first(&library(&[10]), &github).await, None);
    }
}
