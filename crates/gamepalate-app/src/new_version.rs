use std::{env, fmt, path::Path, str::FromStr, time::Duration};

use gamepalate_steam::transport::Fetch;
use reqwest::{Client, Method, Request, Url};
use serde_json::Value;

const RELEASES_URL: &str =
    "https://github.com/cristian-tentella/gamepalate/releases/latest/download";
const RELEASES_URL_VARIABLE: &str = "GAMEPALATE_RELEASES_URL";
const RELEASE_PAGE_URL: &str = "https://github.com/cristian-tentella/gamepalate/releases/tag";
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);
const UNINSTALLER: &str = "unins000.exe";
pub(crate) const SETUP_FILE: &str = "gamepalate-setup.exe";
pub(crate) const GUI_EXE: &str = "gamepalate.exe";
pub(crate) const CLI_EXE: &str = "gamepalate-cli.exe";
pub(crate) const LINUX_TAR: &str = "gamepalate-x86_64-linux.tar.gz";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn current() -> Version {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("the package version has three numbers")
    }
}

impl FromStr for Version {
    type Err = ();

    fn from_str(text: &str) -> Result<Version, ()> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut next = || -> Result<u32, ()> {
            let part = parts.next().ok_or(())?;
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            part.parse().map_err(|_| ())
        };
        let version = Version {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        match parts.next() {
            None => Ok(version),
            Some(_) => Err(()),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    pub version: Version,
    pub release_page: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
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

    #[error("{url} did not answer with a version")]
    Shape { url: String },

    #[error("{url} is not a valid address")]
    Url { url: String },
}

pub fn releases_url() -> String {
    env::var(RELEASES_URL_VARIABLE)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .map_or_else(
            || RELEASES_URL.to_string(),
            |url| url.trim().trim_end_matches('/').to_string(),
        )
}

pub fn release_asset_url(releases_url: &str, file: &str) -> String {
    format!("{}/{file}", releases_url.trim_end_matches('/'))
}

pub async fn check<F: Fetch>(
    fetch: &F,
    releases_url: &str,
) -> Result<Option<NewVersion>, CheckError> {
    check_against(fetch, releases_url, Version::current()).await
}

pub async fn check_releases() -> Result<Option<NewVersion>, CheckError> {
    let releases_url = releases_url();
    let client = Client::builder()
        .build()
        .map_err(|source| CheckError::Network {
            url: releases_url.clone(),
            source,
        })?;
    check(&client, &releases_url).await
}

async fn check_against<F: Fetch>(
    fetch: &F,
    releases_url: &str,
    current: Version,
) -> Result<Option<NewVersion>, CheckError> {
    let url = release_asset_url(releases_url, "version.json");
    let parsed = Url::parse(&url).map_err(|_| CheckError::Url { url: url.clone() })?;
    let mut request = Request::new(Method::GET, parsed);
    *request.timeout_mut() = Some(TIMEOUT);

    let response = fetch
        .fetch(request)
        .await
        .map_err(|source| CheckError::Network {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(CheckError::Status { url, status });
    }
    let text = response
        .text()
        .await
        .map_err(|source| CheckError::Network {
            url: url.clone(),
            source,
        })?;

    let value: Value =
        serde_json::from_str(&text).map_err(|_| CheckError::Shape { url: url.clone() })?;
    let Some(text) = value.get("version").and_then(Value::as_str) else {
        return Err(CheckError::Shape { url });
    };
    let Ok(version) = text.parse::<Version>() else {
        return Ok(None);
    };

    Ok((version > current).then(|| NewVersion {
        version,
        release_page: format!("{RELEASE_PAGE_URL}/v{version}"),
    }))
}

pub fn checksum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        let valid = hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
        (valid && name == file).then(|| hash.to_ascii_lowercase())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installation {
    WindowsSetup,
    WindowsPortable,
    LinuxPortable,
}

impl Installation {
    pub fn detect(exe: &Path) -> Option<Installation> {
        if cfg!(windows) {
            let installed = exe
                .parent()
                .is_some_and(|dir| dir.join(UNINSTALLER).is_file());
            Some(if installed {
                Installation::WindowsSetup
            } else {
                Installation::WindowsPortable
            })
        } else if cfg!(target_os = "linux") {
            Some(Installation::LinuxPortable)
        } else {
            None
        }
    }

    pub fn files(self, exe: &Path) -> Vec<&'static str> {
        match self {
            Installation::WindowsSetup => vec![SETUP_FILE],
            Installation::WindowsPortable => {
                let mut files = vec![GUI_EXE];
                if exe.parent().is_some_and(|dir| dir.join(CLI_EXE).is_file()) {
                    files.push(CLI_EXE);
                }
                files
            }
            Installation::LinuxPortable => vec![LINUX_TAR],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct FakeGithub {
        status: u16,
        body: String,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Fetch for FakeGithub {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<reqwest::Response>> + Send {
            self.asked.lock().unwrap().push(request.url().to_string());
            let response = http::Response::builder()
                .status(self.status)
                .body(self.body.clone())
                .unwrap();
            async move { Ok(response.into()) }
        }
    }

    fn github(status: u16, body: &str) -> (FakeGithub, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let fake = FakeGithub {
            status,
            body: body.to_string(),
            asked: Arc::clone(&asked),
        };
        (fake, asked)
    }

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    async fn run(status: u16, body: &str) -> Result<Option<NewVersion>, CheckError> {
        let (fake, _) = github(status, body);
        check_against(&fake, "https://example.test/dl", version("0.8.0")).await
    }

    #[test]
    fn versions_are_read_from_three_numbers() {
        assert_eq!(
            version("0.9.0"),
            Version {
                major: 0,
                minor: 9,
                patch: 0
            }
        );
        assert_eq!(version("v0.9.0"), version("0.9.0"));
        for bad in ["0.9", "0.9.x", "", "1.2.3.4", "v", "-1.0.0"] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?}");
        }
        assert_eq!(version("0.9.0").to_string(), "0.9.0");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version("0.10.0") > version("0.9.1"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert!(Version::current() >= version("0.0.0"));
    }

    #[tokio::test]
    async fn a_newer_version_is_reported_with_its_release_page() {
        let found = run(200, r#"{"version":"0.9.0"}"#).await.unwrap().unwrap();
        assert_eq!(found.version, version("0.9.0"));
        assert_eq!(
            found.release_page,
            "https://github.com/cristian-tentella/gamepalate/releases/tag/v0.9.0"
        );
    }

    #[tokio::test]
    async fn the_same_or_an_older_version_is_not_new() {
        assert_eq!(run(200, r#"{"version":"0.8.0"}"#).await.unwrap(), None);
        assert_eq!(run(200, r#"{"version":"0.7.9"}"#).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unreadable_version_is_not_new() {
        assert_eq!(run(200, r#"{"version":"soon"}"#).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_failed_request_or_a_wrong_shape_is_an_error() {
        assert!(matches!(
            run(404, "Not Found").await,
            Err(CheckError::Status { .. })
        ));
        assert!(matches!(
            run(200, r#"{"other":"0.9.0"}"#).await,
            Err(CheckError::Shape { .. })
        ));
        assert!(matches!(
            run(200, "<html>").await,
            Err(CheckError::Shape { .. })
        ));
        assert!(matches!(
            run(200, r#"{"version":9}"#).await,
            Err(CheckError::Shape { .. })
        ));
    }

    #[tokio::test]
    async fn the_check_asks_for_version_json_under_the_given_address() {
        let (fake, asked) = github(200, r#"{"version":"0.9.0"}"#);
        check_against(&fake, "http://localhost:8000/dl/", version("0.8.0"))
            .await
            .unwrap();
        assert_eq!(
            *asked.lock().unwrap(),
            vec!["http://localhost:8000/dl/version.json".to_string()]
        );
    }

    #[test]
    fn checksums_are_found_by_file_name() {
        let hash = "a".repeat(64);
        let other = "B".repeat(64);
        let sums = format!(
            "{hash}  gamepalate.exe\nnot a line\n{}  broken.exe\n{other} *gamepalate-cli.exe\n",
            "c".repeat(10)
        );
        assert_eq!(checksum(&sums, "gamepalate.exe"), Some(hash));
        assert_eq!(checksum(&sums, "gamepalate-cli.exe"), Some("b".repeat(64)));
        assert_eq!(checksum(&sums, "broken.exe"), None);
        assert_eq!(checksum(&sums, "missing.exe"), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_installation_follows_the_uninstaller() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(GUI_EXE);
        assert_eq!(
            Installation::detect(&exe),
            Some(Installation::WindowsPortable)
        );
        std::fs::write(dir.path().join(UNINSTALLER), b"").unwrap();
        assert_eq!(Installation::detect(&exe), Some(Installation::WindowsSetup));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_always_portable() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Installation::detect(&dir.path().join("gamepalate")),
            Some(Installation::LinuxPortable)
        );
    }

    #[test]
    fn the_files_to_download_follow_the_installation() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(GUI_EXE);
        assert_eq!(Installation::WindowsSetup.files(&exe), vec![SETUP_FILE]);
        assert_eq!(Installation::WindowsPortable.files(&exe), vec![GUI_EXE]);
        std::fs::write(dir.path().join(CLI_EXE), b"").unwrap();
        assert_eq!(
            Installation::WindowsPortable.files(&exe),
            vec![GUI_EXE, CLI_EXE]
        );
        assert_eq!(Installation::LinuxPortable.files(&exe), vec![LINUX_TAR]);
    }
}
