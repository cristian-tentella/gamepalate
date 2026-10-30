use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use gamepalate_steam::{
    images::{ImageDownloader, ImageError},
    transport::Fetch,
};
use reqwest::Client;
use sha2::{Digest, Sha256};

const EXTENSIONS: [&str; 3] = ["jpg", "png", "webp"];

static WRITES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum LoadImageError {
    #[error("there is no folder for the image cache")]
    NoCacheDir,

    #[error(transparent)]
    Download(#[from] ImageError),

    #[error("could not use the cached image {}", path.display())]
    Disk {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub struct ImageCache<F = Client> {
    dir: Option<PathBuf>,
    downloader: ImageDownloader<F>,
}

impl<F: Fetch> ImageCache<F> {
    pub fn new(dir: Option<PathBuf>, downloader: ImageDownloader<F>) -> ImageCache<F> {
        ImageCache { dir, downloader }
    }

    pub async fn image(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        let dir = self.dir.as_deref().ok_or(LoadImageError::NoCacheDir)?;
        let path = dir.join(file_name(url));

        match fs::read(&path) {
            Ok(bytes) => {
                touch(&path);
                return Ok(bytes);
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(disk(&path, error));
            }
            Err(_) => {}
        }

        let bytes = self.downloader.download(url).await?;
        write(dir, &path, &bytes)?;

        Ok(bytes)
    }
}

fn file_name(url: &str) -> String {
    let hash: String = Sha256::digest(url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path
        .rsplit('/')
        .next()
        .and_then(|last| last.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .filter(|extension| EXTENSIONS.contains(&extension.as_str()));

    match extension {
        Some(extension) => format!("{hash}.{extension}"),
        None => hash,
    }
}

fn touch(path: &Path) {
    let _ = File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()));
}

fn write(dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), LoadImageError> {
    fs::create_dir_all(dir).map_err(|error| disk(dir, error))?;
    let temporary = dir.join(format!(
        "{}.{}-{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default(),
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));

    fs::write(&temporary, bytes).map_err(|error| disk(&temporary, error))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        disk(path, error)
    })
}

fn disk(path: &Path, source: std::io::Error) -> LoadImageError {
    LoadImageError::Disk {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use reqwest::{Request, Response};

    use super::*;

    const URL: &str =
        "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/620/header.jpg";

    struct FakeCdn {
        status: u16,
        calls: Arc<AtomicU32>,
    }

    impl Fetch for FakeCdn {
        fn fetch(
            &self,
            _request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let response = http::Response::builder()
                .status(self.status)
                .header("content-type", "image/jpeg")
                .body(vec![1, 2, 3])
                .unwrap();

            async move { Ok(response.into()) }
        }
    }

    fn cache(dir: Option<&Path>, status: u16) -> (ImageCache<FakeCdn>, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        let fake = FakeCdn {
            status,
            calls: Arc::clone(&calls),
        };
        let cache = ImageCache::new(
            dir.map(|dir| dir.join("images")),
            ImageDownloader::with_fetch(fake),
        );

        (cache, calls)
    }

    fn files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir.join("images"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn downloads_and_saves_an_image_the_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, calls) = cache(Some(dir.path()), 200);

        assert_eq!(cache.image(URL).await.unwrap(), vec![1, 2, 3]);

        assert_eq!(files(dir.path()), vec![file_name(URL)]);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn reads_a_saved_image_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, calls) = cache(Some(dir.path()), 200);
        cache.image(URL).await.unwrap();

        assert_eq!(cache.image(URL).await.unwrap(), vec![1, 2, 3]);

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn leaves_no_file_when_the_download_fails() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, _) = cache(Some(dir.path()), 404);

        assert!(matches!(
            cache.image(URL).await,
            Err(LoadImageError::Download(_))
        ));

        assert!(files(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn fails_without_a_cache_folder() {
        let (cache, calls) = cache(None, 200);

        assert!(matches!(
            cache.image(URL).await,
            Err(LoadImageError::NoCacheDir)
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn names_the_file_after_the_address() {
        let name = file_name(URL);

        assert_eq!(name, file_name(URL));
        assert_eq!(name.len(), 64 + ".jpg".len());
        assert!(name.ends_with(".jpg"));
        assert_ne!(name, file_name("https://example.com/other.jpg"));
    }

    #[test]
    fn takes_the_extension_before_the_query() {
        assert!(file_name("https://example.com/a/shot.PNG?t=123").ends_with(".png"));
        assert!(file_name("https://example.com/a/shot.webp#top").ends_with(".webp"));
    }

    #[test]
    fn leaves_out_an_unknown_extension() {
        assert_eq!(file_name("https://example.com/image").len(), 64);
        assert_eq!(file_name("https://example.com/v1.2/image").len(), 64);
        assert_eq!(file_name("https://example.com/page.html").len(), 64);
    }
}
