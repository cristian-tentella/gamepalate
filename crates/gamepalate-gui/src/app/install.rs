use std::sync::{Arc, Mutex};

use gamepalate_app::{
    install::{SystemLaunch, download_release},
    new_version::check_releases,
    setup::error_chain,
};
use iced::{
    Task,
    futures::{SinkExt, channel::mpsc},
    stream,
};

use super::{App, Message, Start};
use crate::{new_version, screen::Screen};

impl App {
    pub(super) fn update_install(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NewVersionChecked(found) => {
                if let Err(error) = &found {
                    tracing::warn!("could not check for new versions: {error}");
                }
                self.new_versions.checked(found.map_err(drop));
                Task::none()
            }
            Message::NewVersionDownloading(percent) => {
                self.new_versions.downloading(percent);
                Task::none()
            }
            Message::NewVersionDownloaded(Ok(downloaded)) => {
                self.downloaded = downloaded
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                self.new_versions.installing();
                let saving = match &mut self.screen {
                    Screen::Settings(screen) => screen.leave().discard(),
                    _ => Task::none(),
                };
                saving
                    .chain(self.save_window())
                    .chain(Task::done(Message::InstallNewVersion))
            }
            Message::NewVersionDownloaded(Err(problem)) => {
                self.new_versions.failed(problem);
                Task::none()
            }
            Message::InstallNewVersion => {
                let Some(downloaded) = self.downloaded.take() else {
                    return Task::none();
                };
                match downloaded.install(&SystemLaunch, true) {
                    Ok(installed) => {
                        self.installed = Some(installed);
                        iced::exit()
                    }
                    Err(error) => {
                        self.new_versions.failed(new_version::problem(&error));
                        Task::none()
                    }
                }
            }
            _ => Task::none(),
        }
    }

    pub(super) fn check_new_version(&mut self, start: Start) -> Task<Message> {
        if !self.new_versions.start_check(start) {
            return Task::none();
        }

        Task::perform(
            async { check_releases().await.map_err(|error| error_chain(&error)) },
            Message::NewVersionChecked,
        )
    }

    pub(super) fn download_new_version(&mut self) -> Task<Message> {
        let Some(dir) = self
            .gamepalate
            .as_ref()
            .map(|gamepalate| gamepalate.data_dir().to_path_buf())
        else {
            return Task::none();
        };
        let Some((new, target)) = self.new_versions.start_download() else {
            return Task::none();
        };

        Task::run(
            stream::channel(64, async move |mut output: mpsc::Sender<Message>| {
                let mut events = output.clone();
                let mut shown = None;
                let downloaded = download_release(new.version, &target, &dir, move |progress| {
                    let percent = progress.percent();
                    if percent != shown {
                        shown = percent;
                        let _ = events.try_send(Message::NewVersionDownloading(percent));
                    }
                })
                .await;
                let message = Message::NewVersionDownloaded(
                    downloaded
                        .map(|downloaded| Arc::new(Mutex::new(Some(downloaded))))
                        .map_err(|error| new_version::problem(&error)),
                );
                let _ = output.send(message).await;
            }),
            |message| message,
        )
    }
}
