use std::{
    fmt,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use avif_decode::Decoder;
use gamepalate_steam::{
    images::{ImageDownloader, ImageError},
    transport::{Fetch, SteamError},
};
use image::{DynamicImage, ImageBuffer, ImageFormat, RgbaImage};
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

const EXTENSIONS: [&str; 4] = ["jpg", "png", "webp", "avif"];
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

    #[error("could not read the AVIF image")]
    Avif(#[from] avif_decode::Error),

    #[error("could not use the cached image {}", path.display())]
    Disk {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl fmt::Debug for DecodedImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageKind {
    Capsule,
    Cover,
    Screenshot,
    Extra,
}

impl ImageKind {
    const ALL: [ImageKind; 4] = [
        ImageKind::Capsule,
        ImageKind::Cover,
        ImageKind::Screenshot,
        ImageKind::Extra,
    ];

    fn folder(self) -> &'static str {
        match self {
            ImageKind::Capsule => "capsules",
            ImageKind::Cover => "covers",
            ImageKind::Screenshot => "screenshots",
            ImageKind::Extra => "extras",
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

    pub async fn extra(&self, url: &str) -> Result<DecodedImage, LoadImageError> {
        let bytes = self
            .prepared(url, ImageKind::Extra, |bytes| {
                decode_extra(&bytes)?;
                Ok(bytes)
            })
            .await?;
        decode_extra(&bytes)
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

fn decode_extra(bytes: &[u8]) -> Result<DecodedImage, LoadImageError> {
    let image = match image::guess_format(bytes) {
        Ok(ImageFormat::Avif) => std::panic::catch_unwind(|| avif(bytes)).unwrap_or(Err(
            LoadImageError::Avif(avif_decode::Error::InternalError("the decoder stopped")),
        ))?,
        _ => image::load_from_memory(bytes)?.to_rgba8(),
    };
    Ok(DecodedImage {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

fn avif(bytes: &[u8]) -> Result<RgbaImage, LoadImageError> {
    let unsupported = || avif_decode::Error::Unsupported("the image size");
    let image = match Decoder::from_avif(bytes)?.to_image()? {
        avif_decode::Image::Rgb8(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            DynamicImage::ImageRgb8(sized(width, height, raw).ok_or_else(unsupported)?)
        }
        avif_decode::Image::Rgba8(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
            DynamicImage::ImageRgba8(sized(width, height, raw).ok_or_else(unsupported)?)
        }
        avif_decode::Image::Gray8(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().map(|p| p.value()).collect();
            DynamicImage::ImageLuma8(sized(width, height, raw).ok_or_else(unsupported)?)
        }
        avif_decode::Image::Rgb16(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            DynamicImage::ImageRgb16(sized(width, height, raw).ok_or_else(unsupported)?)
        }
        avif_decode::Image::Rgba16(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
            DynamicImage::ImageRgba16(sized(width, height, raw).ok_or_else(unsupported)?)
        }
        avif_decode::Image::Gray16(image) => {
            let (pixels, width, height) = image.into_contiguous_buf();
            let raw = pixels.iter().map(|p| p.value()).collect();
            DynamicImage::ImageLuma16(sized(width, height, raw).ok_or_else(unsupported)?)
        }
    };
    Ok(image.to_rgba8())
}

fn sized<P: image::Pixel>(
    width: usize,
    height: usize,
    raw: Vec<P::Subpixel>,
) -> Option<ImageBuffer<P, Vec<P::Subpixel>>> {
    ImageBuffer::from_raw(width.try_into().ok()?, height.try_into().ok()?, raw)
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
    const RED_AVIF: &str = concat!(
        "00000020667479706176696600000000617669666d6966316d6961664d413142000000f96d657461",
        "000000000000002f68646c7200000000000000007069637400000000000000000000000050696374",
        "75726548616e646c6572000000000e7069746d0000000000010000001e696c6f6300000000440000",
        "01000100000001000001210000001a0000002869696e660000000000010000001a696e6665020000",
        "000001000061763031436f6c6f72000000006a697072700000004b6970636f000000146973706500",
        "0000000000000800000008000000107069786900000000030808080000000c6176314381000c0000",
        "000013636f6c726e636c78000200020002000000001769706d610000000000000001000104010283",
        "04000000226d6461740a051808bf6c023211180000005000000000b013a89c0f03ac80",
    );

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

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    fn close_to(pixel: &[u8], expected: [u8; 4]) -> bool {
        pixel
            .iter()
            .zip(expected)
            .all(|(&got, wanted)| got.abs_diff(wanted) <= 8)
    }

    #[test]
    fn decodes_an_avif() {
        let image = decode_extra(&bytes(RED_AVIF)).unwrap();

        assert_eq!((image.width, image.height), (8, 8));
        let pixel = &image.rgba[(5 * 8 + 3) * 4..][..4];
        assert!(close_to(pixel, [0xc8, 0x32, 0x14, 0xff]));
    }

    #[test]
    fn decodes_a_webp() {
        let original = RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 90, 7, 255])
        });
        let mut webp = Vec::new();
        DynamicImage::ImageRgba8(original.clone())
            .write_to(&mut std::io::Cursor::new(&mut webp), ImageFormat::WebP)
            .unwrap();

        let image = decode_extra(&webp).unwrap();

        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(image.rgba, original.into_raw());
    }

    #[test]
    fn an_image_that_does_not_decode_is_an_error() {
        let mut broken = bytes(RED_AVIF);
        broken.truncate(64);

        assert!(matches!(
            decode_extra(&broken),
            Err(LoadImageError::Avif(_))
        ));
        assert!(matches!(
            decode_extra(b"<html>not found</html>"),
            Err(LoadImageError::Decode(_))
        ));
    }

    struct Cdn(Vec<u8>);

    impl Fetch for Cdn {
        fn fetch(
            &self,
            _request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            let response = http::Response::builder()
                .status(200)
                .header("content-type", "image/avif")
                .body(self.0.clone())
                .unwrap();

            async move { Ok(response.into()) }
        }
    }

    #[tokio::test]
    async fn keeps_an_extra_as_it_came_and_gives_its_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::new(
            Some(dir.path().join("images")),
            ImageDownloader::with_fetch(Cdn(bytes(RED_AVIF))),
        );
        let url = "https://cdn.test/extras/red.avif?t=1";

        let image = cache.extra(url).await.unwrap();

        assert_eq!((image.width, image.height), (8, 8));
        let saved = dir
            .path()
            .join("images")
            .join("extras")
            .join(file_name(url));
        assert!(saved.to_string_lossy().ends_with(".avif"));
        assert_eq!(fs::read(saved).unwrap(), bytes(RED_AVIF));
        assert_eq!(cache.extra(url).await.unwrap(), image);
    }

    #[tokio::test]
    async fn does_not_keep_an_extra_that_does_not_decode() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::new(
            Some(dir.path().join("images")),
            ImageDownloader::with_fetch(Cdn(b"<html>not found</html>".to_vec())),
        );

        assert!(cache.extra("https://cdn.test/extras/x.avif").await.is_err());
        assert_eq!(cache.bytes(), 0);
    }
}
