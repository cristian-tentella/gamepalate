use std::{
    env,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use gamepalate_steam::transport::Fetch;
use reqwest::{Method, Request, Response, StatusCode, Url};
use sha2::{Digest, Sha256};
use tokio::time::timeout;

use crate::{
    lock::{LockError, UpdateLock},
    log::failed,
    new_version::{CLI_EXE, GUI_EXE, Installation, TIMEOUT, Version, checksum, release_asset_url},
};

const SUMS_FILE: &str = "SHA256SUMS.txt";
const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);
const SETUP_DIR: &str = "gamepalate-new-version";
const LINUX_DIR: &str = ".gamepalate-new";
const LINUX_FOLDER: &str = "gamepalate";
const LINUX_GUI: &str = "gamepalate";
const NEW: &str = ".new";
const OLD: &str = ".old";
const SETUP_ARGS: [&str; 3] = ["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"];
const NO_LAUNCH: &str = "/NOLAUNCH";

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the data is being updated")]
    UpdateRunning,

    #[error("could not lock the data folder for the installation")]
    Lock(#[source] io::Error),

    #[error("cannot write in {}", dir.display())]
    NotWritable {
        dir: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not reach {url}")]
    Network {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: StatusCode },

    #[error("{url} stopped answering")]
    TimedOut { url: String },

    #[error("{url} is not a valid address")]
    Url { url: String },

    #[error("{file} is not in SHA256SUMS.txt")]
    NotListed { file: String },

    #[error("{file} does not match its hash in SHA256SUMS.txt")]
    Damaged { file: String },

    #[error("could not write {}", path.display())]
    Disk {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not extract {}: {reason}", archive.display())]
    Tar { archive: PathBuf, reason: String },

    #[error("could not replace {}", path.display())]
    Rename {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not start the setup {}", path.display())]
    Setup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("the new version is installed, but {} did not start", path.display())]
    Restart {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub file: usize,
    pub files: usize,
    pub downloaded: u64,
    pub length: Option<u64>,
}

impl Progress {
    pub fn percent(&self) -> Option<u8> {
        let length = self.length?;
        let part = match length {
            0 => 100,
            _ => self.downloaded.min(length) * 100 / length,
        };
        let done = (self.file as u64 * 100 + part) / self.files.max(1) as u64;
        Some(done.min(100) as u8)
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    installation: Installation,
    exe: PathBuf,
    temp_dir: PathBuf,
}

impl Target {
    pub fn current() -> Option<Target> {
        let exe = env::current_exe().ok()?;
        let installation = Installation::detect(&exe)?;
        Some(Target::new(installation, exe, env::temp_dir()))
    }

    pub fn new(installation: Installation, exe: PathBuf, temp_dir: PathBuf) -> Target {
        Target {
            installation,
            exe,
            temp_dir,
        }
    }

    pub fn installation(&self) -> Installation {
        self.installation
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }

    fn dir(&self) -> &Path {
        self.exe.parent().unwrap_or(Path::new(""))
    }

    fn staging(&self) -> PathBuf {
        match self.installation {
            Installation::WindowsSetup => self.temp_dir.join(SETUP_DIR),
            Installation::WindowsPortable => self.dir().to_path_buf(),
            Installation::LinuxPortable => self.dir().join(LINUX_DIR),
        }
    }

    fn place(&self, file: &str) -> PathBuf {
        match self.installation {
            Installation::WindowsPortable => self.dir().join(format!("{file}{NEW}")),
            Installation::WindowsSetup | Installation::LinuxPortable => self.staging().join(file),
        }
    }

    fn prepare(&self, first: &Path) -> Result<(), InstallError> {
        let staging = self.staging();
        if self.installation != Installation::WindowsPortable {
            let _ = fs::remove_dir_all(&staging);
        }
        fs::create_dir_all(&staging)
            .and_then(|()| File::create(first).map(drop))
            .map_err(|source| match source.kind() {
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                    InstallError::NotWritable {
                        dir: staging,
                        source,
                    }
                }
                _ => disk(first, source),
            })
    }

    fn discard(&self, files: &[(&str, PathBuf)]) {
        match self.installation {
            Installation::WindowsPortable => {
                for (_, path) in files {
                    let _ = fs::remove_file(path);
                }
            }
            Installation::WindowsSetup | Installation::LinuxPortable => {
                let _ = fs::remove_dir_all(self.staging());
            }
        }
    }
}

#[derive(Debug)]
pub struct Downloaded {
    target: Target,
    version: Version,
    files: Vec<(&'static str, PathBuf)>,
    lock: UpdateLock,
}

#[derive(Debug)]
pub struct Installed {
    _lock: UpdateLock,
}

pub trait Launch {
    fn run_setup(&self, setup: &Path, args: &[&str]) -> io::Result<()>;
    fn start(&self, exe: &Path) -> io::Result<()>;
    fn replace(&self, exe: &Path) -> io::Result<()>;
}

pub struct SystemLaunch;

impl Launch for SystemLaunch {
    fn run_setup(&self, setup: &Path, args: &[&str]) -> io::Result<()> {
        detached(Command::new(setup).args(args))
    }

    fn start(&self, exe: &Path) -> io::Result<()> {
        detached(&mut Command::new(exe))
    }

    fn replace(&self, exe: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Err(Command::new(exe).args(env::args_os().skip(1)).exec())
        }
        #[cfg(not(unix))]
        {
            let _ = exe;
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }
}

fn detached(command: &mut Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

pub async fn download<F: Fetch>(
    fetch: &F,
    releases_url: &str,
    version: Version,
    target: &Target,
    data_dir: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<Downloaded, InstallError> {
    let lock = UpdateLock::take(data_dir).map_err(|error| match error {
        LockError::Taken => InstallError::UpdateRunning,
        LockError::Io(source) => InstallError::Lock(source),
    })?;
    let files: Vec<(&'static str, PathBuf)> = target
        .installation
        .files(&target.exe)
        .into_iter()
        .map(|file| (file, target.place(file)))
        .collect();
    if let Some((_, first)) = files.first() {
        target.prepare(first)?;
    }

    tracing::info!("downloading gamepalate {version}");
    match download_files(fetch, releases_url, &files, &mut progress).await {
        Ok(()) => Ok(Downloaded {
            target: target.clone(),
            version,
            files,
            lock,
        }),
        Err(error) => {
            target.discard(&files);
            failed("could not download the new version")(&error);
            Err(error)
        }
    }
}

impl Downloaded {
    pub fn version(&self) -> Version {
        self.version
    }

    pub fn install(self, launcher: &impl Launch, restart: bool) -> Result<Installed, InstallError> {
        let installed = match self.target.installation {
            Installation::WindowsSetup => self.run_setup(launcher, restart),
            Installation::WindowsPortable => self.swap_exes(launcher, restart),
            Installation::LinuxPortable => self.unpack(launcher, restart),
        };
        installed.inspect_err(failed("could not install the new version"))?;
        Ok(Installed { _lock: self.lock })
    }

    fn run_setup(&self, launcher: &impl Launch, restart: bool) -> Result<(), InstallError> {
        let Some((_, setup)) = self.files.first() else {
            return Ok(());
        };
        let mut args = SETUP_ARGS.to_vec();
        if !restart {
            args.push(NO_LAUNCH);
        }
        tracing::info!("starting the setup of gamepalate {}", self.version);
        launcher.run_setup(setup, &args).map_err(|source| {
            self.target.discard(&self.files);
            InstallError::Setup {
                path: setup.clone(),
                source,
            }
        })
    }

    fn swap_exes(&self, launcher: &impl Launch, restart: bool) -> Result<(), InstallError> {
        let dir = self.target.dir();
        let mut swapped = Vec::new();
        for (file, new) in &self.files {
            let current = dir.join(file);
            let old = dir.join(format!("{file}{OLD}"));
            if let Err(source) = swap(&current, &old, new, &mut swapped) {
                put_back(&swapped);
                self.target.discard(&self.files);
                return Err(InstallError::Rename {
                    path: current,
                    source,
                });
            }
        }
        tracing::info!("installed gamepalate {}", self.version);

        if restart {
            let gui = dir.join(GUI_EXE);
            launcher
                .start(&gui)
                .map_err(|source| InstallError::Restart { path: gui, source })?;
        }
        Ok(())
    }

    fn unpack(&self, launcher: &impl Launch, restart: bool) -> Result<(), InstallError> {
        let staging = self.target.staging();
        let replaced = self.replace_from(&staging);
        let _ = fs::remove_dir_all(&staging);
        replaced?;
        tracing::info!("installed gamepalate {}", self.version);

        if restart {
            let gui = self.target.dir().join(LINUX_GUI);
            launcher
                .replace(&gui)
                .map_err(|source| InstallError::Restart { path: gui, source })?;
        }
        Ok(())
    }

    fn replace_from(&self, staging: &Path) -> Result<(), InstallError> {
        let Some((_, archive)) = self.files.first() else {
            return Ok(());
        };
        extract(archive, staging)?;

        let folder = staging.join(LINUX_FOLDER);
        if !folder.join(LINUX_GUI).is_file() {
            return Err(InstallError::Tar {
                archive: archive.clone(),
                reason: format!("the archive has no {LINUX_FOLDER}/{LINUX_GUI}"),
            });
        }
        let extracted: Vec<PathBuf> = fs::read_dir(&folder)
            .map_err(|source| InstallError::Tar {
                archive: archive.clone(),
                reason: source.to_string(),
            })?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect();

        let dir = self.target.dir();
        for path in extracted {
            let Some(name) = path.file_name() else {
                continue;
            };
            let to = dir.join(name);
            fs::rename(&path, &to).map_err(|source| InstallError::Rename { path: to, source })?;
        }
        Ok(())
    }
}

pub fn clean_up(exe: &Path) {
    let Some(dir) = exe.parent() else {
        return;
    };
    for file in [GUI_EXE, CLI_EXE] {
        let _ = fs::remove_file(dir.join(format!("{file}{OLD}")));
    }
    let _ = fs::remove_dir_all(dir.join(LINUX_DIR));
}

async fn download_files<F: Fetch>(
    fetch: &F,
    releases_url: &str,
    files: &[(&str, PathBuf)],
    progress: &mut impl FnMut(Progress),
) -> Result<(), InstallError> {
    let sums = fetch_text(fetch, &release_asset_url(releases_url, SUMS_FILE)).await?;
    let hashes = files
        .iter()
        .map(|(file, _)| {
            checksum(&sums, file).ok_or_else(|| InstallError::NotListed {
                file: file.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (index, ((file, path), hash)) in files.iter().zip(hashes).enumerate() {
        let url = release_asset_url(releases_url, file);
        let mut response = send(fetch, &url).await?;
        let length = response.content_length();
        let mut report = |downloaded| {
            progress(Progress {
                file: index,
                files: files.len(),
                downloaded,
                length,
            })
        };
        report(0);

        let mut out = File::create(path).map_err(|source| disk(path, source))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0;
        while let Some(chunk) = timeout(CHUNK_TIMEOUT, response.chunk())
            .await
            .map_err(|_| timed_out(&url))?
            .map_err(|source| network(&url, source))?
        {
            hasher.update(&chunk);
            out.write_all(&chunk).map_err(|source| disk(path, source))?;
            downloaded += chunk.len() as u64;
            report(downloaded);
        }
        out.sync_all().map_err(|source| disk(path, source))?;

        if hex(&hasher.finalize()) != hash {
            return Err(InstallError::Damaged {
                file: file.to_string(),
            });
        }
    }
    Ok(())
}

async fn send<F: Fetch>(fetch: &F, url: &str) -> Result<Response, InstallError> {
    let parsed = Url::parse(url).map_err(|_| InstallError::Url {
        url: url.to_string(),
    })?;
    let response = timeout(TIMEOUT, fetch.fetch(Request::new(Method::GET, parsed)))
        .await
        .map_err(|_| timed_out(url))?
        .map_err(|source| network(url, source))?;
    let status = response.status();
    if !status.is_success() {
        return Err(InstallError::Status {
            url: url.to_string(),
            status,
        });
    }
    Ok(response)
}

async fn fetch_text<F: Fetch>(fetch: &F, url: &str) -> Result<String, InstallError> {
    let response = send(fetch, url).await?;
    timeout(TIMEOUT, response.text())
        .await
        .map_err(|_| timed_out(url))?
        .map_err(|source| network(url, source))
}

fn swap(
    current: &Path,
    old: &Path,
    new: &Path,
    swapped: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> io::Result<()> {
    let kept = if current.exists() {
        let _ = fs::remove_file(old);
        fs::rename(current, old)?;
        Some(old.to_path_buf())
    } else {
        None
    };
    swapped.push((current.to_path_buf(), kept));
    fs::rename(new, current)
}

fn put_back(swapped: &[(PathBuf, Option<PathBuf>)]) {
    for (current, old) in swapped.iter().rev() {
        let _ = match old {
            Some(old) => fs::rename(old, current),
            None => fs::remove_file(current),
        };
    }
}

fn extract(archive: &Path, into: &Path) -> Result<(), InstallError> {
    let tar_error = |reason: String| InstallError::Tar {
        archive: archive.to_path_buf(),
        reason,
    };
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => tar_error("tar is not installed".to_string()),
            _ => tar_error(error.to_string()),
        })?;
    if !output.status.success() {
        return Err(tar_error(format!(
            "tar ended with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn disk(path: &Path, source: io::Error) -> InstallError {
    InstallError::Disk {
        path: path.to_path_buf(),
        source,
    }
}

fn network(url: &str, source: reqwest::Error) -> InstallError {
    InstallError::Network {
        url: url.to_string(),
        source,
    }
}

fn timed_out(url: &str) -> InstallError {
    InstallError::TimedOut {
        url: url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::new_version::{LINUX_TAR, SETUP_FILE};

    struct FakeRelease {
        files: HashMap<String, Vec<u8>>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Fetch for FakeRelease {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<reqwest::Response>> + Send {
            let url = request.url().to_string();
            self.asked.lock().unwrap().push(url.clone());
            let name = url.rsplit('/').next().unwrap_or_default();
            let response = match self.files.get(name) {
                Some(body) => http::Response::builder()
                    .status(200)
                    .body(body.clone())
                    .unwrap(),
                None => http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap(),
            };
            async move { Ok(response.into()) }
        }
    }

    #[derive(Debug, PartialEq)]
    enum Launched {
        Setup(PathBuf, Vec<String>),
        Start(PathBuf),
        Replace(PathBuf),
    }

    #[derive(Default)]
    struct FakeLaunch {
        launched: Mutex<Vec<Launched>>,
    }

    impl Launch for FakeLaunch {
        fn run_setup(&self, setup: &Path, args: &[&str]) -> io::Result<()> {
            let args = args.iter().map(|arg| arg.to_string()).collect();
            self.record(Launched::Setup(setup.to_path_buf(), args))
        }

        fn start(&self, exe: &Path) -> io::Result<()> {
            self.record(Launched::Start(exe.to_path_buf()))
        }

        fn replace(&self, exe: &Path) -> io::Result<()> {
            self.record(Launched::Replace(exe.to_path_buf()))
        }
    }

    impl FakeLaunch {
        fn record(&self, launched: Launched) -> io::Result<()> {
            self.launched.lock().unwrap().push(launched);
            Ok(())
        }

        fn launched(&self) -> Vec<Launched> {
            std::mem::take(&mut *self.launched.lock().unwrap())
        }
    }

    struct Place {
        root: TempDir,
    }

    impl Place {
        fn new() -> Place {
            let place = Place {
                root: tempfile::tempdir().unwrap(),
            };
            fs::create_dir_all(place.app()).unwrap();
            fs::create_dir_all(place.temp()).unwrap();
            place
        }

        fn app(&self) -> PathBuf {
            self.root.path().join("app")
        }

        fn data(&self) -> PathBuf {
            self.root.path().join("data")
        }

        fn temp(&self) -> PathBuf {
            self.root.path().join("temp")
        }

        fn write(&self, file: &str, body: &str) {
            fs::write(self.app().join(file), body).unwrap();
        }

        fn read(&self, file: &str) -> String {
            fs::read_to_string(self.app().join(file)).unwrap()
        }

        fn files(&self) -> Vec<String> {
            let mut files: Vec<String> = fs::read_dir(self.app())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            files.sort();
            files
        }

        fn target(&self, installation: Installation, exe: &str) -> Target {
            Target::new(installation, self.app().join(exe), self.temp())
        }
    }

    fn sha(body: &[u8]) -> String {
        hex(&Sha256::digest(body))
    }

    fn sums(files: &[(&str, &str)]) -> String {
        files
            .iter()
            .map(|(file, body)| format!("{}  {file}\n", sha(body.as_bytes())))
            .collect()
    }

    fn release(files: &[(&str, &str)], sums: &str) -> (FakeRelease, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mut served: HashMap<String, Vec<u8>> = files
            .iter()
            .map(|(file, body)| (file.to_string(), body.as_bytes().to_vec()))
            .collect();
        served.insert(SUMS_FILE.to_string(), sums.as_bytes().to_vec());
        let fake = FakeRelease {
            files: served,
            asked: Arc::clone(&asked),
        };
        (fake, asked)
    }

    fn honest_release(files: &[(&str, &str)]) -> FakeRelease {
        release(files, &sums(files)).0
    }

    fn version() -> Version {
        "0.9.0".parse().unwrap()
    }

    async fn fetched(
        fake: &FakeRelease,
        target: &Target,
        place: &Place,
    ) -> Result<Downloaded, InstallError> {
        download(
            fake,
            "https://example.test/dl",
            version(),
            target,
            &place.data(),
            |_| {},
        )
        .await
    }

    const NEW_FILES: [(&str, &str); 2] = [(GUI_EXE, "new gui"), (CLI_EXE, "new cli")];

    fn portable_place() -> Place {
        let place = Place::new();
        place.write(GUI_EXE, "old gui");
        place.write(CLI_EXE, "old cli");
        place
    }

    #[test]
    fn the_percent_counts_every_file_the_same() {
        let progress = |file, downloaded, length| Progress {
            file,
            files: 2,
            downloaded,
            length,
        };
        assert_eq!(progress(0, 0, Some(10)).percent(), Some(0));
        assert_eq!(progress(0, 5, Some(10)).percent(), Some(25));
        assert_eq!(progress(1, 5, Some(10)).percent(), Some(75));
        assert_eq!(progress(1, 10, Some(10)).percent(), Some(100));
        assert_eq!(progress(1, 0, Some(0)).percent(), Some(100));
        assert_eq!(progress(0, 5, None).percent(), None);
    }

    #[tokio::test]
    async fn a_portable_download_lands_next_to_the_exe() {
        let place = portable_place();
        let (fake, asked) = release(&NEW_FILES, &sums(&NEW_FILES));
        let target = place.target(Installation::WindowsPortable, GUI_EXE);
        let mut reports = Vec::new();

        let downloaded = download(
            &fake,
            "https://example.test/dl",
            version(),
            &target,
            &place.data(),
            |progress| reports.push(progress),
        )
        .await
        .unwrap();

        assert_eq!(downloaded.version(), version());
        assert_eq!(place.read("gamepalate.exe.new"), "new gui");
        assert_eq!(place.read("gamepalate-cli.exe.new"), "new cli");
        assert_eq!(place.read(GUI_EXE), "old gui");
        assert_eq!(place.read(CLI_EXE), "old cli");
        assert_eq!(
            *asked.lock().unwrap(),
            [
                "https://example.test/dl/SHA256SUMS.txt",
                "https://example.test/dl/gamepalate.exe",
                "https://example.test/dl/gamepalate-cli.exe",
            ]
        );
        assert_eq!(reports.first().and_then(Progress::percent), Some(0));
        assert_eq!(reports.last().and_then(Progress::percent), Some(100));
        assert_eq!(reports.last().unwrap().length, Some(7));
    }

    #[tokio::test]
    async fn the_setup_is_downloaded_in_the_temporary_folder() {
        let place = Place::new();
        place.write(GUI_EXE, "old gui");
        place.write("unins000.exe", "");
        let fake = honest_release(&[(SETUP_FILE, "setup")]);
        let target = place.target(Installation::WindowsSetup, GUI_EXE);

        fetched(&fake, &target, &place).await.unwrap();

        let setup = place.temp().join(SETUP_DIR).join(SETUP_FILE);
        assert_eq!(fs::read_to_string(setup).unwrap(), "setup");
        assert_eq!(place.files(), [GUI_EXE, "unins000.exe"].map(String::from));
    }

    #[tokio::test]
    async fn the_linux_archive_is_downloaded_next_to_the_binaries() {
        let place = Place::new();
        place.write("gamepalate", "old gui");
        let fake = honest_release(&[(LINUX_TAR, "archive")]);
        let target = place.target(Installation::LinuxPortable, "gamepalate");

        fetched(&fake, &target, &place).await.unwrap();

        let archive = place.app().join(LINUX_DIR).join(LINUX_TAR);
        assert_eq!(fs::read_to_string(archive).unwrap(), "archive");
    }

    #[tokio::test]
    async fn a_wrong_hash_leaves_nothing_behind() {
        let place = portable_place();
        let wrong = sums(&[(GUI_EXE, "new gui"), (CLI_EXE, "something else")]);
        let (fake, _) = release(&NEW_FILES, &wrong);
        let target = place.target(Installation::WindowsPortable, GUI_EXE);

        let error = fetched(&fake, &target, &place).await.unwrap_err();

        assert!(matches!(error, InstallError::Damaged { file } if file == CLI_EXE));
        assert_eq!(place.files(), [CLI_EXE, GUI_EXE].map(String::from));
        assert_eq!(place.read(GUI_EXE), "old gui");
        assert_eq!(place.read(CLI_EXE), "old cli");
    }

    #[tokio::test]
    async fn a_wrong_hash_removes_the_linux_staging_folder() {
        let place = Place::new();
        place.write("gamepalate", "old gui");
        let (fake, _) = release(&[(LINUX_TAR, "archive")], &sums(&[(LINUX_TAR, "other")]));
        let target = place.target(Installation::LinuxPortable, "gamepalate");

        let error = fetched(&fake, &target, &place).await.unwrap_err();

        assert!(matches!(error, InstallError::Damaged { .. }));
        assert_eq!(place.files(), ["gamepalate"].map(String::from));
    }

    #[tokio::test]
    async fn a_file_missing_from_the_checksums_is_not_downloaded() {
        let place = portable_place();
        let (fake, asked) = release(&NEW_FILES, &sums(&NEW_FILES[..1]));
        let target = place.target(Installation::WindowsPortable, GUI_EXE);

        let error = fetched(&fake, &target, &place).await.unwrap_err();

        assert!(matches!(error, InstallError::NotListed { file } if file == CLI_EXE));
        assert_eq!(
            *asked.lock().unwrap(),
            ["https://example.test/dl/SHA256SUMS.txt"]
        );
        assert_eq!(place.files(), [CLI_EXE, GUI_EXE].map(String::from));
        assert_eq!(place.read(GUI_EXE), "old gui");
    }

    #[tokio::test]
    async fn a_file_missing_from_the_release_leaves_nothing_behind() {
        let place = portable_place();
        let (fake, _) = release(&NEW_FILES[..1], &sums(&NEW_FILES));
        let target = place.target(Installation::WindowsPortable, GUI_EXE);

        let error = fetched(&fake, &target, &place).await.unwrap_err();

        assert!(matches!(
            error,
            InstallError::Status { status, .. } if status == StatusCode::NOT_FOUND
        ));
        assert_eq!(place.files(), [CLI_EXE, GUI_EXE].map(String::from));
    }

    #[tokio::test]
    async fn a_running_update_stops_the_download_before_it_starts() {
        let place = portable_place();
        let (fake, asked) = release(&NEW_FILES, &sums(&NEW_FILES));
        let target = place.target(Installation::WindowsPortable, GUI_EXE);
        let _held = UpdateLock::take(&place.data()).unwrap();

        let error = fetched(&fake, &target, &place).await.unwrap_err();

        assert!(matches!(error, InstallError::UpdateRunning));
        assert!(asked.lock().unwrap().is_empty());
        assert_eq!(place.files(), [CLI_EXE, GUI_EXE].map(String::from));
    }

    #[tokio::test]
    async fn the_portable_exes_are_swapped_and_the_new_one_started() {
        let place = portable_place();
        let fake = honest_release(&NEW_FILES);
        let target = place.target(Installation::WindowsPortable, GUI_EXE);
        let launcher = FakeLaunch::default();

        let downloaded = fetched(&fake, &target, &place).await.unwrap();
        let _installed = downloaded.install(&launcher, true).unwrap();

        assert_eq!(place.read(GUI_EXE), "new gui");
        assert_eq!(place.read("gamepalate.exe.old"), "old gui");
        assert_eq!(place.read(CLI_EXE), "new cli");
        assert_eq!(place.read("gamepalate-cli.exe.old"), "old cli");
        assert_eq!(
            place.files(),
            [
                CLI_EXE,
                "gamepalate-cli.exe.old",
                GUI_EXE,
                "gamepalate.exe.old"
            ]
            .map(String::from)
        );
        assert_eq!(
            launcher.launched(),
            [Launched::Start(place.app().join(GUI_EXE))]
        );
    }

    #[tokio::test]
    async fn a_portable_gui_alone_is_swapped_alone() {
        let place = Place::new();
        place.write(GUI_EXE, "old gui");
        let fake = honest_release(&NEW_FILES);
        let target = place.target(Installation::WindowsPortable, GUI_EXE);
        let launcher = FakeLaunch::default();

        let downloaded = fetched(&fake, &target, &place).await.unwrap();
        let _installed = downloaded.install(&launcher, false).unwrap();

        assert_eq!(place.read(GUI_EXE), "new gui");
        assert_eq!(place.read("gamepalate.exe.old"), "old gui");
        assert_eq!(
            place.files(),
            [GUI_EXE, "gamepalate.exe.old"].map(String::from)
        );
        assert!(launcher.launched().is_empty());
    }

    #[tokio::test]
    async fn a_rename_that_fails_halfway_puts_everything_back() {
        let place = portable_place();
        fs::create_dir(place.app().join("gamepalate-cli.exe.old")).unwrap();
        let fake = honest_release(&NEW_FILES);
        let target = place.target(Installation::WindowsPortable, GUI_EXE);
        let launcher = FakeLaunch::default();

        let downloaded = fetched(&fake, &target, &place).await.unwrap();
        let error = downloaded.install(&launcher, true).unwrap_err();

        assert!(matches!(
            error,
            InstallError::Rename { path, .. } if path == place.app().join(CLI_EXE)
        ));
        assert_eq!(place.read(GUI_EXE), "old gui");
        assert_eq!(place.read(CLI_EXE), "old cli");
        assert_eq!(
            place.files(),
            [CLI_EXE, "gamepalate-cli.exe.old", GUI_EXE].map(String::from)
        );
        assert!(launcher.launched().is_empty());
    }

    async fn installed_setup(restart: bool) -> (Place, Installed, Vec<Launched>) {
        let place = Place::new();
        place.write("unins000.exe", "");
        let fake = honest_release(&[(SETUP_FILE, "setup")]);
        let target = place.target(Installation::WindowsSetup, GUI_EXE);
        let launcher = FakeLaunch::default();

        let installed = fetched(&fake, &target, &place)
            .await
            .unwrap()
            .install(&launcher, restart)
            .unwrap();
        (place, installed, launcher.launched())
    }

    fn setup_args(extra: &[&str]) -> Vec<String> {
        SETUP_ARGS
            .iter()
            .chain(extra)
            .map(|arg| arg.to_string())
            .collect()
    }

    #[tokio::test]
    async fn the_setup_is_started_silently_and_the_lock_is_kept() {
        let (place, _installed, launched) = installed_setup(true).await;

        let setup = place.temp().join(SETUP_DIR).join(SETUP_FILE);
        assert_eq!(launched, [Launched::Setup(setup, setup_args(&[]))]);
        assert!(matches!(
            UpdateLock::take(&place.data()),
            Err(LockError::Taken)
        ));
    }

    #[tokio::test]
    async fn the_setup_without_a_restart_does_not_launch_the_app() {
        let (place, _installed, launched) = installed_setup(false).await;

        let setup = place.temp().join(SETUP_DIR).join(SETUP_FILE);
        assert_eq!(launched, [Launched::Setup(setup, setup_args(&[NO_LAUNCH]))]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_linux_archive_replaces_the_files_and_keeps_them_executable() {
        use std::os::unix::fs::PermissionsExt;

        let place = Place::new();
        place.write("gamepalate", "old gui");
        place.write("gamepalate-cli", "old cli");
        let source = place.root.path().join("source");
        let folder = source.join(LINUX_FOLDER);
        fs::create_dir_all(&folder).unwrap();
        for (file, body) in [
            ("gamepalate", "new gui"),
            ("gamepalate-cli", "new cli"),
            ("gamepalate.desktop", "[Desktop Entry]"),
        ] {
            fs::write(folder.join(file), body).unwrap();
        }
        for file in ["gamepalate", "gamepalate-cli"] {
            fs::set_permissions(folder.join(file), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let archive = place.root.path().join(LINUX_TAR);
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg(LINUX_FOLDER)
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = fs::read(&archive).unwrap();
        let (mut fake, _) = release(&[], "");
        fake.files.insert(
            SUMS_FILE.to_string(),
            format!("{}  {LINUX_TAR}\n", sha(&bytes)).into_bytes(),
        );
        fake.files.insert(LINUX_TAR.to_string(), bytes);
        let target = place.target(Installation::LinuxPortable, "gamepalate");
        let launcher = FakeLaunch::default();

        let downloaded = fetched(&fake, &target, &place).await.unwrap();
        let _installed = downloaded.install(&launcher, true).unwrap();

        assert_eq!(place.read("gamepalate"), "new gui");
        assert_eq!(place.read("gamepalate-cli"), "new cli");
        assert_eq!(place.read("gamepalate.desktop"), "[Desktop Entry]");
        for file in ["gamepalate", "gamepalate-cli"] {
            let mode = fs::metadata(place.app().join(file))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{file}");
        }
        assert!(!place.app().join(LINUX_DIR).exists());
        assert_eq!(
            launcher.launched(),
            [Launched::Replace(place.app().join("gamepalate"))]
        );
    }

    #[test]
    fn the_clean_up_removes_what_the_last_installation_left() {
        let place = portable_place();
        place.write("gamepalate.exe.old", "old gui");
        place.write("gamepalate-cli.exe.old", "old cli");
        place.write("other.exe.old", "not ours");
        fs::create_dir_all(place.app().join(LINUX_DIR).join(LINUX_FOLDER)).unwrap();
        place.write(&format!("{LINUX_DIR}/{LINUX_TAR}"), "archive");

        clean_up(&place.app().join(GUI_EXE));

        assert_eq!(
            place.files(),
            [CLI_EXE, GUI_EXE, "other.exe.old"].map(String::from)
        );
        clean_up(&place.app().join(GUI_EXE));
        assert_eq!(
            place.files(),
            [CLI_EXE, GUI_EXE, "other.exe.old"].map(String::from)
        );
    }
}
