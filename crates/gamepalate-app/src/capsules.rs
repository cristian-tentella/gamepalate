use std::{collections::HashMap, fmt, sync::Arc};

use futures::lock::Mutex;
use gamepalate_core::{ids::AppId, region::Region};
use gamepalate_steam::{
    details::get_capsule_urls,
    images::{ImageError, capsule_url},
    transport::{Fetch, Get, SteamError, Transport},
};
use image::imageops::FilterType;
use reqwest::StatusCode;

use crate::images::{ImageCache, LoadImageError};

const LANGUAGE: &str = "english";
pub const CAPSULE_WIDTH: u32 = 300;
pub const CAPSULE_HEIGHT: u32 = 450;

#[derive(Clone, PartialEq, Eq)]
pub struct Capsule {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl fmt::Debug for Capsule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Capsule")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

pub(crate) struct CapsuleFinder<S = Transport> {
    transport: S,
    region: Region,
    ids: Vec<AppId>,
    found: Mutex<Option<Arc<HashMap<AppId, String>>>>,
}

impl<S> fmt::Debug for CapsuleFinder<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapsuleFinder")
            .finish_non_exhaustive()
    }
}

impl<S: Get + Sync> CapsuleFinder<S> {
    pub(crate) fn new(transport: S, region: Region, ids: Vec<AppId>) -> CapsuleFinder<S> {
        CapsuleFinder {
            transport,
            region,
            ids,
            found: Mutex::new(None),
        }
    }

    async fn known(&self, id: AppId) -> Option<String> {
        self.found.lock().await.as_ref()?.get(&id).cloned()
    }

    async fn find(&self, id: AppId) -> Result<Option<String>, SteamError> {
        let mut found = self.found.lock().await;
        if found.is_none() {
            let urls = get_capsule_urls(&self.transport, &self.ids, LANGUAGE, self.region.as_str())
                .await?;
            *found = Some(Arc::new(urls));
        }
        Ok(found.as_ref().and_then(|urls| urls.get(&id).cloned()))
    }
}

pub(crate) async fn capsule<F: Fetch, S: Get + Sync>(
    id: AppId,
    images: &ImageCache<F>,
    finder: &CapsuleFinder<S>,
) -> Result<Capsule, LoadImageError> {
    let bytes = match finder.known(id).await {
        Some(url) => images.image(&url).await?,
        None => match images.image(&capsule_url(id)).await {
            Err(error) if is_not_found(&error) => match finder.find(id).await? {
                Some(url) => images.image(&url).await?,
                None => return Err(error),
            },
            result => result?,
        },
    };
    Ok(downscale(&bytes)?)
}

pub(crate) async fn classic_capsule<F: Fetch>(
    id: AppId,
    images: &ImageCache<F>,
) -> Result<Capsule, LoadImageError> {
    Ok(downscale(&images.image(&capsule_url(id)).await?)?)
}

fn is_not_found(error: &LoadImageError) -> bool {
    matches!(
        error,
        LoadImageError::Download(ImageError::Status {
            status: StatusCode::NOT_FOUND,
            ..
        })
    )
}

fn downscale(bytes: &[u8]) -> Result<Capsule, image::ImageError> {
    let image = image::load_from_memory(bytes)?.to_rgba8();
    let small =
        image::imageops::resize(&image, CAPSULE_WIDTH, CAPSULE_HEIGHT, FilterType::Triangle);
    Ok(Capsule {
        width: small.width(),
        height: small.height(),
        rgba: small.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Mutex as StdMutex};

    use gamepalate_steam::images::ImageDownloader;
    use reqwest::{Request, Response};
    use serde_json::{Value, json};

    use super::*;

    struct FakeCdn {
        files: HashMap<String, Vec<u8>>,
        asked: Arc<StdMutex<Vec<String>>>,
    }

    impl Fetch for FakeCdn {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            let url = request.url().to_string();
            self.asked.lock().unwrap().push(url.clone());
            let response = match self.files.get(&url) {
                Some(body) => http::Response::builder()
                    .status(200)
                    .header("content-type", "image/png")
                    .body(body.clone()),
                None => http::Response::builder()
                    .status(404)
                    .header("content-type", "text/html")
                    .body(Vec::new()),
            }
            .unwrap();

            async move { Ok(response.into()) }
        }
    }

    struct FakeSteam {
        capsules: Vec<(u32, &'static str)>,
        calls: StdMutex<u32>,
    }

    impl Get for FakeSteam {
        fn get(
            &self,
            _endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            *self.calls.lock().unwrap() += 1;
            let items: Vec<Value> = self
                .capsules
                .iter()
                .map(|&(id, file)| {
                    json!({ "id": id, "success": 1, "assets": {
                    "asset_url_format": format!("steam/apps/{id}/${{FILENAME}}"),
                    "library_capsule": file,
                } })
                })
                .collect();
            async move { Ok(json!({ "response": { "store_items": items } })) }
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn asset(path: &str) -> String {
        format!("https://shared.fastly.steamstatic.com/store_item_assets/{path}")
    }

    struct Setup {
        _dir: tempfile::TempDir,
        images: ImageCache<FakeCdn>,
        finder: CapsuleFinder<FakeSteam>,
        cdn_asked: Arc<StdMutex<Vec<String>>>,
    }

    fn setup(files: Vec<(String, Vec<u8>)>, capsules: Vec<(u32, &'static str)>) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let cdn_asked = Arc::new(StdMutex::new(Vec::new()));
        let images = ImageCache::new(
            Some(dir.path().join("images")),
            ImageDownloader::with_fetch(FakeCdn {
                files: files.into_iter().collect(),
                asked: Arc::clone(&cdn_asked),
            }),
        );
        let finder = CapsuleFinder::new(
            FakeSteam {
                capsules,
                calls: StdMutex::new(0),
            },
            Region::ITALY,
            vec![AppId(1), AppId(2), AppId(3)],
        );
        Setup {
            _dir: dir,
            images,
            finder,
            cdn_asked,
        }
    }

    impl Setup {
        async fn capsule(&self, id: u32) -> Result<Capsule, LoadImageError> {
            capsule(AppId(id), &self.images, &self.finder).await
        }

        fn steam_calls(&self) -> u32 {
            *self.finder.transport.calls.lock().unwrap()
        }

        fn cdn_calls(&self) -> usize {
            self.cdn_asked.lock().unwrap().len()
        }
    }

    #[tokio::test]
    async fn the_plain_capsule_is_reduced_to_half() {
        let setup = setup(
            vec![(asset("steam/apps/1/library_600x900.jpg"), png(600, 900))],
            Vec::new(),
        );

        let capsule = setup.capsule(1).await.unwrap();

        assert_eq!((capsule.width, capsule.height), (300, 450));
        assert_eq!(capsule.rgba.len(), 300 * 450 * 4);
        assert_eq!(&capsule.rgba[..4], [200, 30, 30, 255]);
        assert_eq!(setup.steam_calls(), 0);
    }

    #[tokio::test]
    async fn a_missing_capsule_is_found_once_through_steam() {
        let setup = setup(
            vec![
                (
                    asset("steam/apps/2/7a0f/library_capsule.jpg"),
                    png(600, 900),
                ),
                (asset("steam/apps/3/portrait.png"), png(600, 900)),
            ],
            vec![(2, "7a0f/library_capsule.jpg"), (3, "portrait.png")],
        );

        assert_eq!(setup.capsule(2).await.unwrap().width, 300);
        assert_eq!(setup.capsule(3).await.unwrap().width, 300);
        assert_eq!(setup.capsule(2).await.unwrap().width, 300);

        assert_eq!(setup.steam_calls(), 1);
        assert_eq!(setup.cdn_calls(), 3);
    }

    #[tokio::test]
    async fn without_any_capsule_the_first_error_stays() {
        let setup = setup(Vec::new(), Vec::new());

        let error = setup.capsule(1).await.unwrap_err();

        assert!(is_not_found(&error));
        assert_eq!(setup.steam_calls(), 1);
    }

    #[tokio::test]
    async fn the_classic_capsule_never_asks_steam() {
        let setup = setup(
            vec![(asset("steam/apps/1/library_600x900.jpg"), png(600, 900))],
            vec![(2, "7a0f/library_capsule.jpg")],
        );

        assert_eq!(
            classic_capsule(AppId(1), &setup.images)
                .await
                .unwrap()
                .width,
            300
        );
        assert!(is_not_found(
            &classic_capsule(AppId(2), &setup.images).await.unwrap_err()
        ));
        assert_eq!(setup.steam_calls(), 0);
    }

    #[tokio::test]
    async fn what_is_not_an_image_cannot_be_read() {
        let setup = setup(
            vec![(asset("steam/apps/1/library_600x900.jpg"), vec![1, 2, 3])],
            Vec::new(),
        );

        assert!(matches!(
            setup.capsule(1).await,
            Err(LoadImageError::Decode(_))
        ));
    }
}
