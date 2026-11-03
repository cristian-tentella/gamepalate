use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use gamepalate_steam::{
    images::{ImageDownloader, ImageError},
    transport::{Fetch, SteamError},
};
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

const EXTENSIONS: [&str; 3] = ["jpg", "png", "webp"];
pub(crate) const MAX_CACHE_BYTES: u64 = 300 * 1024 * 1024;
const PRUNE_EVERY: u64 = 20;
const MAX_DOWNLOADS: usize = 4;
const TEMPORARY: &str = ".tmp";

static WRITES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum LoadImageError {
    #[error("there is no folder for the image cache")]
    NoCacheDir,

    #[error(transparent)]
    Download(#[from] ImageError),

    #[error(transparent)]
    Steam(#[from] SteamError),

    #[error("could not read the image")]
    Decode(#[from] image::ImageError),

    #[error("could not use the cached image {}", path.display())]
    Disk {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageKind {
    Capsule,
    Cover,
    Screenshot,
}

impl ImageKind {
    const ALL: [ImageKind; 3] = [ImageKind::Capsule, ImageKind::Cover, ImageKind::Screenshot];

    fn folder(self) -> &'static str {
        match self {
            ImageKind::Capsule => "capsules",
            ImageKind::Cover => "covers",
            ImageKind::Screenshot => "screenshots",
        }
    }
}

#[derive(Debug)]
pub struct ImageCache<F = Client> {
    dir: Option<PathBuf>,
    downloader: ImageDownloader<F>,
    limit: u64,
    written: AtomicU64,
    downloads: Semaphore,
}

pub(crate) fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("gamepalate").join("cache").join("images"))
}

impl<F: Fetch> ImageCache<F> {
    pub fn new(dir: Option<PathBuf>, downloader: ImageDownloader<F>) -> ImageCache<F> {
        ImageCache {
            dir,
            downloader,
            limit: MAX_CACHE_BYTES,
            written: AtomicU64::new(0),
            downloads: Semaphore::new(MAX_DOWNLOADS),
        }
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>, LoadImageError> {
        Ok(self.downloader.download(url).await?)
    }

    pub async fn image(&self, url: &str, kind: ImageKind) -> Result<Vec<u8>, LoadImageError> {
        self.prepared(url, kind, Ok).await
    }

    pub(crate) async fn prepared(
        &self,
        url: &str,
        kind: ImageKind,
        prepare: impl FnOnce(Vec<u8>) -> Result<Vec<u8>, LoadImageError>,
    ) -> Result<Vec<u8>, LoadImageError> {
        let dir = self
            .dir
            .as_deref()
            .ok_or(LoadImageError::NoCacheDir)?
            .join(kind.folder());
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

        let bytes = {
            let _download = self.downloads.acquire().await;
            self.downloader.download(url).await?
        };
        let bytes = prepare(bytes)?;
        write(&dir, &path, &bytes)?;
        if (self.written.fetch_add(1, Ordering::Relaxed) + 1).is_multiple_of(PRUNE_EVERY) {
            self.prune();
        }

        Ok(bytes)
    }
}

impl<F> ImageCache<F> {
    pub fn prune(&self) {
        if let Some(dir) = &self.dir {
            prune(dir, self.limit);
        }
    }

    pub fn bytes(&self) -> u64 {
        self.dir.as_deref().map_or(0, |dir| {
            folder_bytes(dir)
                + ImageKind::ALL
                    .iter()
                    .map(|kind| folder_bytes(&dir.join(kind.folder())))
                    .sum::<u64>()
        })
    }

    pub fn clear(&self) {
        if let Some(dir) = &self.dir {
            prune(dir, 0);
        }
    }
}

fn folder_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

struct CachedFile {
    path: PathBuf,
    kind: Option<ImageKind>,
    size: u64,
    used_at: SystemTime,
}

fn prune(dir: &Path, limit: u64) {
    let mut files = cached_files(dir, None);
    for kind in ImageKind::ALL {
        files.extend(cached_files(&dir.join(kind.folder()), Some(kind)));
    }

    for path in to_delete(files, limit) {
        let _ = fs::remove_file(path);
    }
}

fn cached_files(dir: &Path, kind: Option<ImageKind>) -> Vec<CachedFile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry
                .metadata()
                .ok()
                .filter(|metadata| metadata.is_file())?;
            Some(CachedFile {
                path: entry.path(),
                kind,
                size: metadata.len(),
                used_at: metadata.modified().ok()?,
            })
        })
        .collect()
}

fn to_delete(files: Vec<CachedFile>, limit: u64) -> Vec<PathBuf> {
    let (temporary, mut images): (Vec<_>, Vec<_>) = files
        .into_iter()
        .partition(|file| file.kind.is_none() || file.path.to_string_lossy().ends_with(TEMPORARY));
    images.sort_by_key(|file| (file.kind, std::cmp::Reverse(file.used_at)));

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
        fs::read_dir(dir.join("images").join("covers"))
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

        assert_eq!(
            cache.image(URL, ImageKind::Cover).await.unwrap(),
            vec![1, 2, 3]
        );

        assert_eq!(files(dir.path()), vec![file_name(URL)]);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    struct SlowCdn {
        running: Arc<AtomicU32>,
        most: Arc<AtomicU32>,
    }

    impl Fetch for SlowCdn {
        fn fetch(
            &self,
            _request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            let running = Arc::clone(&self.running);
            let most = Arc::clone(&self.most);

            async move {
                let now = running.fetch_add(1, Ordering::Relaxed) + 1;
                most.fetch_max(now, Ordering::Relaxed);
                for _ in 0..20 {
                    tokio::task::yield_now().await;
                }
                running.fetch_sub(1, Ordering::Relaxed);
                let response = http::Response::builder()
                    .status(200)
                    .header("content-type", "image/jpeg")
                    .body(vec![1, 2, 3])
                    .unwrap();
                Ok(response.into())
            }
        }
    }

    #[tokio::test]
    async fn downloads_at_most_four_images_at_once_but_reads_the_saved_ones_freely() {
        let dir = tempfile::tempdir().unwrap();
        let most = Arc::new(AtomicU32::new(0));
        let cache = Arc::new(ImageCache::new(
            Some(dir.path().join("images")),
            ImageDownloader::with_fetch(SlowCdn {
                running: Arc::new(AtomicU32::new(0)),
                most: Arc::clone(&most),
            }),
        ));
        let mut loads = tokio::task::JoinSet::new();
        for id in 0..12 {
            let cache = Arc::clone(&cache);
            loads.spawn(async move {
                cache
                    .image(&format!("https://cdn.test/{id}.jpg"), ImageKind::Cover)
                    .await
                    .unwrap()
            });
        }

        while let Some(bytes) = loads.join_next().await {
            assert_eq!(bytes.unwrap(), vec![1, 2, 3]);
        }

        assert_eq!(most.load(Ordering::Relaxed), MAX_DOWNLOADS as u32);
        assert_eq!(cache.downloads.available_permits(), MAX_DOWNLOADS);
    }

    #[tokio::test]
    async fn reads_a_saved_image_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, calls) = cache(Some(dir.path()), 200);
        cache.image(URL, ImageKind::Cover).await.unwrap();

        assert_eq!(
            cache.image(URL, ImageKind::Cover).await.unwrap(),
            vec![1, 2, 3]
        );

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn leaves_no_file_when_the_download_fails() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, _) = cache(Some(dir.path()), 404);

        assert!(matches!(
            cache.image(URL, ImageKind::Cover).await,
            Err(LoadImageError::Download(_))
        ));

        assert!(files(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_download_without_cache_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, calls) = cache(Some(dir.path()), 200);

        assert_eq!(cache.download(URL).await.unwrap(), vec![1, 2, 3]);
        assert_eq!(cache.download(URL).await.unwrap(), vec![1, 2, 3]);

        assert_eq!(cache.bytes(), 0);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn fails_without_a_cache_folder() {
        let (cache, calls) = cache(None, 200);

        assert!(matches!(
            cache.image(URL, ImageKind::Cover).await,
            Err(LoadImageError::NoCacheDir)
        ));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    fn cached(name: &str, size: u64, seconds_ago: u64) -> CachedFile {
        CachedFile {
            path: PathBuf::from(name),
            kind: Some(ImageKind::Cover),
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
    fn deletes_the_screenshots_before_the_covers() {
        let mut shot = cached("shot.jpg", 40, 1);
        shot.kind = Some(ImageKind::Screenshot);
        let files = vec![shot, cached("cover.jpg", 40, 30), cached("old.jpg", 40, 40)];

        assert_eq!(deleted(files, 80), vec!["shot.jpg"]);
    }

    #[test]
    fn deletes_the_files_left_by_the_old_layout() {
        let mut loose = cached("loose.jpg", 10, 1);
        loose.kind = None;

        assert_eq!(
            deleted(vec![loose, cached("a.jpg", 10, 2)], 100),
            vec!["loose.jpg"]
        );
    }

    #[tokio::test]
    async fn keeps_to_the_limit_while_it_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, _) = cache(Some(dir.path()), 200);
        let cache = ImageCache { limit: 30, ..cache };

        for index in 0..PRUNE_EVERY {
            cache
                .image(
                    &format!("https://example.com/{index}.jpg"),
                    ImageKind::Screenshot,
                )
                .await
                .unwrap();
        }

        assert_eq!(cache.bytes(), 30);
    }

    #[tokio::test]
    async fn keeps_covers_and_screenshots_in_their_own_folders() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, _) = cache(Some(dir.path()), 200);

        cache.image(URL, ImageKind::Cover).await.unwrap();
        cache
            .image("https://example.com/ss_1.jpg", ImageKind::Screenshot)
            .await
            .unwrap();

        assert_eq!(files(dir.path()), vec![file_name(URL)]);
        assert!(
            dir.path()
                .join("images")
                .join("screenshots")
                .join(file_name("https://example.com/ss_1.jpg"))
                .is_file()
        );
    }

    #[test]
    fn prunes_the_folder_down_to_the_most_recent_files() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let covers = dir.path().join("covers");
        fs::create_dir(&covers).unwrap();
        for (name, seconds_ago) in [("old.jpg", 30), ("middle.jpg", 20), ("new.jpg", 10)] {
            let path = covers.join(name);
            fs::write(&path, [0; 40]).unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - Duration::from_secs(seconds_ago))
                .unwrap();
        }

        prune(dir.path(), 100);

        let mut left: Vec<String> = fs::read_dir(&covers)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["middle.jpg", "new.jpg"]);
    }

    #[tokio::test]
    async fn counts_the_bytes_of_the_cache_and_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let (cache, _) = cache(Some(dir.path()), 200);
        assert_eq!(cache.bytes(), 0);

        cache.image(URL, ImageKind::Cover).await.unwrap();
        cache
            .image("https://example.com/other.jpg", ImageKind::Cover)
            .await
            .unwrap();
        assert_eq!(cache.bytes(), 6);

        cache.clear();
        assert_eq!(cache.bytes(), 0);
        assert!(files(dir.path()).is_empty());
    }

    #[test]
    fn a_cache_without_a_folder_is_empty() {
        let (cache, _) = cache(None, 200);

        assert_eq!(cache.bytes(), 0);
        cache.clear();
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
