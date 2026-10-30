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
const MAX_CACHE_BYTES: u64 = 300 * 1024 * 1024;
const TEMPORARY: &str = ".tmp";

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

impl<F> ImageCache<F> {
    pub fn prune(&self) {
        if let Some(dir) = &self.dir {
            prune(dir, MAX_CACHE_BYTES);
        }
    }
}

struct CachedFile {
    path: PathBuf,
    size: u64,
    used_at: SystemTime,
}

fn prune(dir: &Path, limit: u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry
                .metadata()
                .ok()
                .filter(|metadata| metadata.is_file())?;
            Some(CachedFile {
                path: entry.path(),
                size: metadata.len(),
                used_at: metadata.modified().ok()?,
            })
        })
        .collect();

    for path in to_delete(files, limit) {
        let _ = fs::remove_file(path);
    }
}

fn to_delete(files: Vec<CachedFile>, limit: u64) -> Vec<PathBuf> {
    let (temporary, mut images): (Vec<_>, Vec<_>) = files
        .into_iter()
        .partition(|file| file.path.to_string_lossy().ends_with(TEMPORARY));
    images.sort_by_key(|file| std::cmp::Reverse(file.used_at));

    let mut kept = 0;
    let mut full = false;
    let old = images.into_iter().filter(|file| {
        full = full || kept + file.size > limit;
        if !full {
            kept += file.size;
        }
        full
    });

    temporary
        .into_iter()
        .chain(old)
        .map(|file| file.path)
        .collect()
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
        "{}.{}-{}{TEMPORARY}",
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
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
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

    fn cached(name: &str, size: u64, seconds_ago: u64) -> CachedFile {
        CachedFile {
            path: PathBuf::from(name),
            size,
            used_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 - seconds_ago),
        }
    }

    fn deleted(files: Vec<CachedFile>, limit: u64) -> Vec<String> {
        to_delete(files, limit)
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn keeps_everything_under_the_limit() {
        let files = vec![cached("a.jpg", 40, 1), cached("b.jpg", 60, 2)];

        assert!(deleted(files, 100).is_empty());
    }

    #[test]
    fn deletes_the_least_recently_used_over_the_limit() {
        let files = vec![
            cached("old.jpg", 40, 30),
            cached("new.jpg", 40, 10),
            cached("middle.jpg", 40, 20),
        ];

        assert_eq!(deleted(files, 100), vec!["old.jpg"]);
    }

    #[test]
    fn deletes_a_small_old_file_after_one_that_does_not_fit() {
        let files = vec![
            cached("new.jpg", 60, 1),
            cached("big.jpg", 60, 2),
            cached("small.jpg", 10, 3),
        ];

        assert_eq!(deleted(files, 100), vec!["big.jpg", "small.jpg"]);
    }

    #[test]
    fn always_deletes_the_temporary_files() {
        let files = vec![cached("a.jpg", 10, 1), cached("a.jpg.1-0.tmp", 10, 2)];

        assert_eq!(deleted(files, 100), vec!["a.jpg.1-0.tmp"]);
    }

    #[test]
    fn prunes_the_folder_down_to_the_most_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for (name, seconds_ago) in [("old.jpg", 30), ("middle.jpg", 20), ("new.jpg", 10)] {
            let path = dir.path().join(name);
            fs::write(&path, [0; 40]).unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - Duration::from_secs(seconds_ago))
                .unwrap();
        }

        prune(dir.path(), 100);

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["middle.jpg", "new.jpg"]);
    }

    #[test]
    fn prunes_nothing_without_a_folder() {
        let dir = tempfile::tempdir().unwrap();

        prune(&dir.path().join("missing"), 100);
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
