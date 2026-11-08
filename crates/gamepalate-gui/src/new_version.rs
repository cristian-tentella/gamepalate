use gamepalate_app::{
    install::{InstallError, Target},
    new_version::{NewVersion, Version},
    setup::error_chain,
};

use crate::app::Start;

#[derive(Debug)]
pub struct NewVersions {
    state: State,
    target: Option<Target>,
    check_at_startup: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum State {
    #[default]
    Unchecked,
    Checking,
    Latest,
    CheckFailed,
    Available(NewVersion),
    Downloading(NewVersion, Option<u8>),
    Installing(NewVersion),
    Failed(NewVersion, Problem),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    UpdateRunning,
    Damaged,
    NotWritable(String),
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Offer {
    Install,
    OpenReleasePage,
}

impl NewVersions {
    pub fn new(target: Option<Target>, check_at_startup: bool) -> NewVersions {
        NewVersions {
            state: State::Unchecked,
            target,
            check_at_startup,
        }
    }

    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    pub fn check_at_startup(&self) -> bool {
        self.check_at_startup
    }

    pub fn set_check_at_startup(&mut self, check: bool) {
        self.check_at_startup = check;
    }

    pub fn available(&self) -> Option<&NewVersion> {
        match &self.state {
            State::Available(new)
            | State::Downloading(new, _)
            | State::Installing(new)
            | State::Failed(new, _) => Some(new),
            State::Unchecked | State::Checking | State::Latest | State::CheckFailed => None,
        }
    }

    fn busy(&self) -> bool {
        matches!(
            self.state,
            State::Checking | State::Downloading(..) | State::Installing(_)
        )
    }

    pub fn start_check(&mut self, start: Start) -> bool {
        if self.busy() || (start == Start::Startup && !self.check_at_startup) {
            return false;
        }
        self.state = State::Checking;
        true
    }

    pub fn checked(&mut self, found: Result<Option<NewVersion>, ()>) {
        if self.state != State::Checking {
            return;
        }
        self.state = match found {
            Ok(Some(new)) => State::Available(new),
            Ok(None) => State::Latest,
            Err(()) => State::CheckFailed,
        };
    }

    pub fn start_download(&mut self) -> Option<(NewVersion, Target)> {
        let (Offer::Install, true) = self.offer()? else {
            return None;
        };
        let new = self.available()?.clone();
        let target = self.target.clone()?;
        self.state = State::Downloading(new.clone(), None);
        Some((new, target))
    }

    pub fn downloading(&mut self, percent: Option<u8>) {
        if let State::Downloading(_, shown) = &mut self.state {
            *shown = percent;
        }
    }

    pub fn installing(&mut self) {
        if let State::Downloading(new, _) = &self.state {
            self.state = State::Installing(new.clone());
        }
    }

    pub fn failed(&mut self, problem: Problem) {
        if let Some(new) = self.available().cloned() {
            self.state = State::Failed(new, problem);
        }
    }

    pub fn version_text(&self) -> String {
        version_text(&self.state, Version::current())
    }

    pub fn offer(&self) -> Option<(Offer, bool)> {
        self.available()?;
        let installable = self.target.is_some()
            && !matches!(self.state, State::Failed(_, Problem::NotWritable(_)));
        let offer = if installable {
            Offer::Install
        } else {
            Offer::OpenReleasePage
        };
        Some((offer, !self.busy()))
    }

    pub fn problem_text(&self) -> Option<String> {
        let State::Failed(new, problem) = &self.state else {
            return None;
        };
        Some(match problem {
            Problem::UpdateRunning => "Wait until the update finishes, then install.".to_string(),
            Problem::Damaged => "The download is damaged. Try again later.".to_string(),
            Problem::NotWritable(dir) => {
                format!("Can't write to {dir}. Download the new version from the release page.")
            }
            Problem::Other(error) => {
                format!("Couldn't install gamepalate {}: {error}", new.version)
            }
        })
    }
}

pub fn problem(error: &InstallError) -> Problem {
    match error {
        InstallError::UpdateRunning => Problem::UpdateRunning,
        InstallError::Damaged { .. } | InstallError::NotListed { .. } => Problem::Damaged,
        InstallError::NotWritable { dir, .. } => Problem::NotWritable(dir.display().to_string()),
        error => Problem::Other(error_chain(error)),
    }
}

fn version_text(state: &State, current: Version) -> String {
    let status = match state {
        State::Unchecked => return format!("gamepalate {current}"),
        State::Checking => "Checking for new versions…".to_string(),
        State::Latest => "You have the latest version".to_string(),
        State::CheckFailed => "Couldn't check for new versions".to_string(),
        State::Available(new) | State::Failed(new, _) => format!("{} is available", new.version),
        State::Downloading(new, Some(percent)) => {
            format!("Downloading {}… {percent}%", new.version)
        }
        State::Downloading(new, None) => format!("Downloading {}…", new.version),
        State::Installing(new) => format!("Installing {}…", new.version),
    };
    format!("gamepalate {current} · {status}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gamepalate_app::new_version::Installation;

    use super::*;

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn new_version() -> NewVersion {
        NewVersion {
            version: version("0.9.0"),
            release_page: "https://github.com/cristian-tentella/gamepalate/releases/tag/v0.9.0"
                .to_string(),
        }
    }

    fn target() -> Option<Target> {
        Some(Target::new(
            Installation::WindowsPortable,
            PathBuf::from("gamepalate.exe"),
            std::env::temp_dir(),
        ))
    }

    fn with(state: State, target: Option<Target>) -> NewVersions {
        NewVersions {
            state,
            target,
            check_at_startup: true,
        }
    }

    #[test]
    fn the_startup_checks_only_when_the_setting_is_on() {
        let mut off = NewVersions::new(target(), false);
        assert!(!off.start_check(Start::Startup));
        assert_eq!(off.state, State::Unchecked);

        let mut on = NewVersions::new(target(), true);
        assert!(on.start_check(Start::Startup));
        assert_eq!(on.state, State::Checking);
    }

    #[test]
    fn check_now_checks_even_when_the_startup_does_not() {
        let mut off = NewVersions::new(target(), false);

        assert!(off.start_check(Start::Button));
        assert_eq!(off.state, State::Checking);
    }

    #[test]
    fn nothing_is_checked_while_a_check_or_a_download_runs() {
        let mut checking = with(State::Checking, target());
        assert!(!checking.start_check(Start::Button));

        let mut downloading = with(State::Downloading(new_version(), Some(40)), target());
        assert!(!downloading.start_check(Start::Button));
        assert_eq!(
            downloading.state,
            State::Downloading(new_version(), Some(40))
        );
    }

    #[test]
    fn the_check_ends_in_one_of_three_states() {
        let mut versions = with(State::Checking, target());
        versions.checked(Ok(Some(new_version())));
        assert_eq!(versions.state, State::Available(new_version()));

        let mut versions = with(State::Checking, target());
        versions.checked(Ok(None));
        assert_eq!(versions.state, State::Latest);

        let mut versions = with(State::Checking, target());
        versions.checked(Err(()));
        assert_eq!(versions.state, State::CheckFailed);
    }

    #[test]
    fn the_version_line_says_each_state() {
        let current = version("0.8.0");
        let line = |state: State| version_text(&state, current);

        assert_eq!(line(State::Unchecked), "gamepalate 0.8.0");
        assert_eq!(
            line(State::Checking),
            "gamepalate 0.8.0 · Checking for new versions…"
        );
        assert_eq!(
            line(State::Latest),
            "gamepalate 0.8.0 · You have the latest version"
        );
        assert_eq!(
            line(State::CheckFailed),
            "gamepalate 0.8.0 · Couldn't check for new versions"
        );
        assert_eq!(
            line(State::Available(new_version())),
            "gamepalate 0.8.0 · 0.9.0 is available"
        );
        assert_eq!(
            line(State::Downloading(new_version(), Some(40))),
            "gamepalate 0.8.0 · Downloading 0.9.0… 40%"
        );
        assert_eq!(
            line(State::Downloading(new_version(), None)),
            "gamepalate 0.8.0 · Downloading 0.9.0…"
        );
        assert_eq!(
            line(State::Installing(new_version())),
            "gamepalate 0.8.0 · Installing 0.9.0…"
        );
        assert_eq!(
            line(State::Failed(new_version(), Problem::Damaged)),
            "gamepalate 0.8.0 · 0.9.0 is available"
        );
    }

    #[test]
    fn the_button_is_off_while_the_new_version_downloads() {
        assert_eq!(
            with(State::Available(new_version()), target()).offer(),
            Some((Offer::Install, true))
        );
        assert_eq!(
            with(State::Downloading(new_version(), Some(40)), target()).offer(),
            Some((Offer::Install, false))
        );
        assert_eq!(with(State::Latest, target()).offer(), None);
    }

    #[test]
    fn a_folder_that_cannot_be_written_offers_the_release_page() {
        let mut versions = with(State::Downloading(new_version(), Some(0)), target());

        versions.failed(Problem::NotWritable(
            "C:\\Program Files\\gamepalate".to_string(),
        ));

        assert_eq!(versions.offer(), Some((Offer::OpenReleasePage, true)));
        assert_eq!(
            versions.problem_text().as_deref(),
            Some(
                "Can't write to C:\\Program Files\\gamepalate. Download the new version from the release page."
            )
        );
        assert!(versions.start_download().is_none());
    }

    #[test]
    fn without_an_installation_only_the_release_page_is_offered() {
        let versions = with(State::Available(new_version()), None);

        assert_eq!(versions.offer(), Some((Offer::OpenReleasePage, true)));
    }

    #[test]
    fn other_problems_keep_the_install_button() {
        let mut versions = with(State::Downloading(new_version(), Some(10)), target());

        versions.failed(Problem::UpdateRunning);

        assert_eq!(versions.offer(), Some((Offer::Install, true)));
        assert_eq!(
            versions.problem_text().as_deref(),
            Some("Wait until the update finishes, then install.")
        );
    }

    #[test]
    fn each_problem_has_its_words() {
        let text = |problem| with(State::Failed(new_version(), problem), target()).problem_text();

        assert_eq!(
            text(Problem::Damaged).as_deref(),
            Some("The download is damaged. Try again later.")
        );
        assert_eq!(
            text(Problem::Other(
                "could not replace gamepalate.exe".to_string()
            ))
            .as_deref(),
            Some("Couldn't install gamepalate 0.9.0: could not replace gamepalate.exe")
        );
    }

    #[test]
    fn install_errors_become_problems() {
        assert_eq!(
            problem(&InstallError::UpdateRunning),
            Problem::UpdateRunning
        );
        assert_eq!(
            problem(&InstallError::Damaged {
                file: "gamepalate.exe".to_string()
            }),
            Problem::Damaged
        );
        assert_eq!(
            problem(&InstallError::NotListed {
                file: "gamepalate.exe".to_string()
            }),
            Problem::Damaged
        );
        assert!(matches!(
            problem(&InstallError::NotWritable {
                dir: PathBuf::from("dir"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            }),
            Problem::NotWritable(dir) if dir == "dir"
        ));
    }

    #[test]
    fn the_download_starts_only_from_the_install_button() {
        let mut versions = with(State::Available(new_version()), target());

        let started = versions.start_download();

        assert_eq!(started.map(|(new, _)| new), Some(new_version()));
        assert_eq!(versions.state, State::Downloading(new_version(), None));
        assert!(versions.start_download().is_none());

        versions.downloading(Some(40));
        assert_eq!(versions.state, State::Downloading(new_version(), Some(40)));
        versions.installing();
        assert_eq!(versions.state, State::Installing(new_version()));
    }
}
