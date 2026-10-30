use gamepalate_core::ids::AppId;
use reqwest::{
    Client, Method, Request, Response, StatusCode, Url,
    header::{ACCEPT, CONTENT_TYPE, HeaderValue},
};
use tokio::time::timeout;

use crate::transport::{Fetch, TIMEOUT, USER_AGENT};

pub(crate) const ASSETS_BASE_URL: &str = "https://shared.fastly.steamstatic.com/store_item_assets/";
const MAX_BYTES: usize = 10 * 1024 * 1024;

pub fn header_url(id: AppId) -> String {
    format!("{ASSETS_BASE_URL}steam/apps/{}/header.jpg", id.0)
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("{url} is not a valid image address")]
    Url { url: String },

    #[error("could not download {url}: {reason}")]
    Network {
        url: String,
        reason: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: StatusCode },

    #[error("{url} is not an image ({})", content_type.as_deref().unwrap_or("no content type"))]
    NotAnImage {
        url: String,
        content_type: Option<String>,
    },

    #[error("{url} is larger than {} MB", MAX_BYTES / 1024 / 1024)]
    TooLarge { url: String },

    #[error("could not set up the HTTP client for images")]
    Client(#[source] reqwest::Error),
}

#[derive(Debug)]
pub struct ImageDownloader<F = Client> {
    fetch: F,
}

impl ImageDownloader {
    pub fn new() -> Result<ImageDownloader, ImageError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(ImageError::Client)?;

        Ok(ImageDownloader::with_fetch(client))
    }
}

impl<F: Fetch> ImageDownloader<F> {
    pub fn with_fetch(fetch: F) -> ImageDownloader<F> {
        ImageDownloader { fetch }
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        match timeout(TIMEOUT, self.download_now(url)).await {
            Ok(result) => result,
            Err(_) => Err(ImageError::Network {
                url: url.to_string(),
                reason: "timed out".to_string(),
                source: None,
            }),
        }
    }

    async fn download_now(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        let parsed = Url::parse(url).map_err(|_| ImageError::Url {
            url: url.to_string(),
        })?;
        let mut request = Request::new(Method::GET, parsed);
        request
            .headers_mut()
            .insert(ACCEPT, HeaderValue::from_static("image/*"));

        let mut response = self
            .fetch
            .fetch(request)
            .await
            .map_err(|error| network(url, error))?;

        check(url, &response)?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| network(url, error))?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(ImageError::TooLarge {
                    url: url.to_string(),
                });
            }
            bytes.extend_from_slice(&chunk);
        }

        Ok(bytes)
    }
}

fn check(url: &str, response: &Response) -> Result<(), ImageError> {
    let status = response.status();
    if !status.is_success() {
        return Err(ImageError::Status {
            url: url.to_string(),
            status,
        });
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if !content_type
        .as_deref()
        .is_some_and(|content_type| content_type.starts_with("image/"))
    {
        return Err(ImageError::NotAnImage {
            url: url.to_string(),
            content_type,
        });
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err(ImageError::TooLarge {
            url: url.to_string(),
        });
    }

    Ok(())
}

fn network(url: &str, error: reqwest::Error) -> ImageError {
    ImageError::Network {
        url: url.to_string(),
        reason: "no answer".to_string(),
        source: Some(error.without_url()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const URL: &str =
        "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/620/header.jpg";

    struct FakeCdn {
        status: u16,
        content_type: Option<&'static str>,
        body: Vec<u8>,
        requests: Mutex<Vec<Request>>,
    }

    impl Fetch for FakeCdn {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            self.requests.lock().unwrap().push(request);
            let mut response = http::Response::builder().status(self.status);
            if let Some(content_type) = self.content_type {
                response = response.header(CONTENT_TYPE, content_type);
            }
            let response = response.body(self.body.clone()).unwrap();

            async move { Ok(response.into()) }
        }
    }

    fn downloader(
        status: u16,
        content_type: Option<&'static str>,
        body: Vec<u8>,
    ) -> ImageDownloader<FakeCdn> {
        ImageDownloader::with_fetch(FakeCdn {
            status,
            content_type,
            body,
            requests: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn returns_the_bytes_of_an_image() {
        let downloader = downloader(200, Some("image/jpeg"), vec![1, 2, 3]);

        assert_eq!(downloader.download(URL).await.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn asks_the_given_address_for_an_image() {
        let downloader = downloader(200, Some("image/jpeg"), vec![1]);

        downloader.download(URL).await.unwrap();

        let requests = downloader.fetch.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url().as_str(), URL);
        assert_eq!(requests[0].headers()[ACCEPT], "image/*");
    }

    #[tokio::test]
    async fn fails_on_a_missing_image() {
        let downloader = downloader(404, Some("text/html"), Vec::new());

        assert!(matches!(
            downloader.download(URL).await,
            Err(ImageError::Status {
                status: StatusCode::NOT_FOUND,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn refuses_what_is_not_an_image() {
        let html = downloader(200, Some("text/html"), b"<html>".to_vec());
        let untyped = downloader(200, None, vec![1]);

        assert!(matches!(
            html.download(URL).await,
            Err(ImageError::NotAnImage { .. })
        ));
        assert!(matches!(
            untyped.download(URL).await,
            Err(ImageError::NotAnImage {
                content_type: None,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn refuses_an_image_over_ten_megabytes() {
        let downloader = downloader(200, Some("image/png"), vec![0; MAX_BYTES + 1]);

        assert!(matches!(
            downloader.download(URL).await,
            Err(ImageError::TooLarge { .. })
        ));
    }

    #[test]
    fn builds_the_header_address_from_the_app_id() {
        assert_eq!(header_url(AppId(620)), URL);
    }
}
