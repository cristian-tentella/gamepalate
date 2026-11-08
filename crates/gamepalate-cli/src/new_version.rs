use std::{io::Write, path::Path};

use anyhow::bail;
use clap::Args;
use gamepalate_app::{
    install::{InstallError, Launch, Progress, SystemLaunch, Target, download},
    new_version::{Installation, NewVersion, Version, check, releases_url},
};
use gamepalate_steam::transport::Fetch;

#[derive(Args)]
pub struct NewVersionArgs {
    /// Install the new version, if there is one
    #[arg(long)]
    install: bool,
}

pub fn run(args: &NewVersionArgs, data_dir: &Path) -> anyhow::Result<()> {
    let client = reqwest::Client::builder().build()?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(execute(
        &client,
        &releases_url(),
        Target::current(),
        data_dir,
        args.install,
        &SystemLaunch,
        &mut std::io::stdout(),
    ))
}

async fn execute<F: Fetch>(
    fetch: &F,
    releases_url: &str,
    target: Option<Target>,
    data_dir: &Path,
    install: bool,
    launcher: &impl Launch,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let found = check(fetch, releases_url)
        .await
        .map_err(|error| anyhow::anyhow!("Couldn't check for new versions: {error}"))?;
    let Some(new) = found else {
        writeln!(out, "You have the latest version, {}.", Version::current())?;
        return Ok(());
    };
    if !install {
        writeln!(
            out,
            "gamepalate {} is available (you have {}). Run gamepalate-cli new-version --install to install it.",
            new.version,
            Version::current()
        )?;
        return Ok(());
    }

    let Some(target) = target else {
        bail!("{}", download_by_hand(&new));
    };
    let mut line = ProgressLine::new(std::io::stderr());
    let downloaded = download(
        fetch,
        releases_url,
        new.version,
        &target,
        data_dir,
        |progress| line.show(progress),
    )
    .await;
    line.finish();
    downloaded
        .map_err(|error| install_error(error, &new))?
        .install(launcher, false)
        .map_err(|error| install_error(error, &new))?;
    match target.installation() {
        Installation::WindowsSetup => writeln!(
            out,
            "Installing gamepalate {} in the background.",
            new.version
        )?,
        _ => {
            writeln!(out, "Installed gamepalate {}.", new.version)?;
            target.clean_up();
        }
    }
    Ok(())
}

struct ProgressLine<W: Write> {
    out: W,
    shown: Option<String>,
}

impl<W: Write> ProgressLine<W> {
    fn new(out: W) -> ProgressLine<W> {
        ProgressLine { out, shown: None }
    }

    fn show(&mut self, progress: Progress) {
        let line = progress_text(progress);
        if self.shown.as_ref() == Some(&line) {
            return;
        }
        let _ = write!(self.out, "\r\x1b[2K{line}");
        let _ = self.out.flush();
        self.shown = Some(line);
    }

    fn finish(mut self) -> W {
        if self.shown.is_some() {
            let _ = write!(self.out, "\r\x1b[2K");
            let _ = self.out.flush();
        }
        self.out
    }
}

fn progress_text(progress: Progress) -> String {
    let file = match progress.files {
        0 | 1 => String::new(),
        files => format!(" {} of {files}", progress.file + 1),
    };
    match progress.length {
        Some(0) => format!("Downloading{file}: 100%"),
        Some(length) => format!(
            "Downloading{file}: {}%",
            progress.downloaded.min(length) * 100 / length
        ),
        None => format!("Downloading{file}: {} bytes", progress.downloaded),
    }
}

fn download_by_hand(new: &NewVersion) -> String {
    format!(
        "Download gamepalate {} from {}.",
        new.version, new.release_page
    )
}

fn install_error(error: InstallError, new: &NewVersion) -> anyhow::Error {
    match error {
        InstallError::UpdateRunning => anyhow::anyhow!("Another update is running."),
        InstallError::NotWritable { dir, .. } => anyhow::anyhow!(
            "Can't write to {}. {}",
            dir.display(),
            download_by_hand(new)
        ),
        InstallError::Damaged { .. } | InstallError::NotListed { .. } => {
            anyhow::anyhow!("The download is damaged. Try again later.")
        }
        error => anyhow::Error::new(error).context("could not install the new version"),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf};

    use reqwest::{Request, Response};
    use sha2::{Digest, Sha256};

    use super::*;

    struct FakeRelease {
        files: HashMap<String, Vec<u8>>,
    }

    impl Fetch for FakeRelease {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            let name = request.url().path().rsplit('/').next().unwrap_or_default();
            let response = match self.files.get(name) {
                Some(body) => http::Response::builder().status(200).body(body.clone()),
                None => http::Response::builder().status(404).body(Vec::new()),
            }
            .unwrap();
            async move { Ok(response.into()) }
        }
    }

    struct NoLaunch;

    impl Launch for NoLaunch {
        fn run_setup(&self, _: &Path, _: &[&str]) -> std::io::Result<()> {
            Ok(())
        }

        fn start(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }

        fn replace(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn release(version: &str) -> FakeRelease {
        let hash: String = Sha256::digest(b"new gui")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let files = [
            ("version.json", format!(r#"{{"version":"{version}"}}"#)),
            ("SHA256SUMS.txt", format!("{hash}  gamepalate.exe\n")),
            ("gamepalate.exe", "new gui".to_string()),
        ];
        FakeRelease {
            files: files
                .into_iter()
                .map(|(name, body)| (name.to_string(), body.into_bytes()))
                .collect(),
        }
    }

    async fn ran(
        fake: &FakeRelease,
        target: Option<Target>,
        data_dir: &Path,
        install: bool,
    ) -> (anyhow::Result<()>, String) {
        let mut out = Vec::new();
        let result = execute(
            fake,
            "https://example.test/dl",
            target,
            data_dir,
            install,
            &NoLaunch,
            &mut out,
        )
        .await;
        (result, String::from_utf8(out).unwrap())
    }

    fn portable(root: &Path) -> (Target, PathBuf) {
        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("gamepalate.exe"), "old gui").unwrap();
        let exe = app.join("gamepalate-cli.exe");
        (
            Target::new(Installation::WindowsPortable, exe, root.join("temp")),
            app,
        )
    }

    #[tokio::test]
    async fn a_new_version_is_announced_with_the_command_to_install_it() {
        let dir = tempfile::tempdir().unwrap();

        let (result, out) = ran(&release("99.0.0"), None, dir.path(), false).await;

        result.unwrap();
        assert_eq!(
            out,
            format!(
                "gamepalate 99.0.0 is available (you have {}). Run gamepalate-cli new-version --install to install it.\n",
                Version::current()
            )
        );
    }

    #[tokio::test]
    async fn the_latest_version_is_confirmed() {
        let dir = tempfile::tempdir().unwrap();

        let (result, out) = ran(&release("0.0.1"), None, dir.path(), true).await;

        result.unwrap();
        assert_eq!(
            out,
            format!("You have the latest version, {}.\n", Version::current())
        );
    }

    #[tokio::test]
    async fn a_failed_check_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let nothing = FakeRelease {
            files: HashMap::new(),
        };

        let (result, out) = ran(&nothing, None, dir.path(), false).await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.starts_with("Couldn't check for new versions: "),
            "{error}"
        );
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn installing_replaces_the_portable_files() {
        let root = tempfile::tempdir().unwrap();
        let (target, app) = portable(root.path());

        let (result, out) = ran(
            &release("99.0.0"),
            Some(target),
            &root.path().join("data"),
            true,
        )
        .await;

        result.unwrap();
        assert_eq!(out, "Installed gamepalate 99.0.0.\n");
        assert_eq!(
            fs::read_to_string(app.join("gamepalate.exe")).unwrap(),
            "new gui"
        );
    }

    #[tokio::test]
    async fn a_damaged_download_touches_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (target, app) = portable(root.path());
        let mut fake = release("99.0.0");
        fake.files
            .insert("gamepalate.exe".to_string(), b"tampered".to_vec());

        let (result, _) = ran(&fake, Some(target), &root.path().join("data"), true).await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "The download is damaged. Try again later."
        );
        assert_eq!(
            fs::read_to_string(app.join("gamepalate.exe")).unwrap(),
            "old gui"
        );
    }

    fn progress(file: usize, files: usize, downloaded: u64, length: Option<u64>) -> Progress {
        Progress {
            file,
            files,
            downloaded,
            length,
        }
    }

    #[test]
    fn the_progress_counts_each_file_on_its_own() {
        assert_eq!(
            progress_text(progress(0, 1, 40, Some(100))),
            "Downloading: 40%"
        );
        assert_eq!(
            progress_text(progress(1, 2, 13, Some(13))),
            "Downloading 2 of 2: 100%"
        );
        assert_eq!(
            progress_text(progress(0, 2, 0, Some(0))),
            "Downloading 1 of 2: 100%"
        );
        assert_eq!(
            progress_text(progress(0, 1, 2048, None)),
            "Downloading: 2048 bytes"
        );
    }

    #[test]
    fn the_progress_line_is_written_only_when_it_changes() {
        let mut line = ProgressLine::new(Vec::new());

        line.show(progress(0, 1, 10, Some(1000)));
        line.show(progress(0, 1, 15, Some(1000)));
        line.show(progress(0, 1, 20, Some(1000)));
        let out = String::from_utf8(line.finish()).unwrap();

        assert_eq!(
            out,
            "\r\x1b[2KDownloading: 1%\r\x1b[2KDownloading: 2%\r\x1b[2K"
        );
    }

    #[test]
    fn finishing_without_progress_writes_nothing() {
        let line = ProgressLine::new(Vec::new());

        assert!(line.finish().is_empty());
    }
}
