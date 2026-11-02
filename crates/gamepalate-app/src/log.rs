use std::{
    backtrace::Backtrace,
    error::Error,
    fs::{self, File},
    io::{self, Write},
    panic,
    path::Path,
    sync::Mutex,
};

use tracing::Level;
use tracing_subscriber::{Layer, filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{data_dir, setup::error_chain};

const FILE: &str = "gamepalate.log";
const PREVIOUS: &str = "gamepalate.previous.log";
const STEAM_ID_DIGITS: usize = 17;
const API_KEY_DIGITS: usize = 32;

pub fn start_log() -> io::Result<()> {
    let dir = data_dir::resolve()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user data folder"))?
        .join("logs");
    let file = open_log(&dir)?;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(Redacted(file)))
                .with_filter(
                    Targets::new()
                        .with_default(Level::WARN)
                        .with_target("gamepalate", Level::INFO),
                ),
        )
        .try_init()
        .map_err(io::Error::other)?;

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        tracing::error!("{info}\n{}", Backtrace::force_capture());
        default_hook(info);
    }));

    Ok(())
}

pub(crate) fn failed<E: Error + 'static>(what: &'static str) -> impl FnOnce(&E) {
    move |error| tracing::warn!("{what}: {}", error_chain(error))
}

fn open_log(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    let path = dir.join(FILE);
    if path.is_file() {
        fs::rename(&path, dir.join(PREVIOUS))?;
    }
    File::create(path)
}

struct Redacted<W>(W);

impl<W: Write> Write for Redacted<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .write_all(redact(&String::from_utf8_lossy(buf)).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn redact(text: &str) -> String {
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(|c: char| c.is_ascii_alphanumeric()) {
        let (before, word) = rest.split_at(start);
        redacted.push_str(before);
        let end = word
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(word.len());
        let (word, after) = word.split_at(end);

        if redacted.ends_with("key=") {
            redacted.push_str("[api key]");
        } else if word.len() == STEAM_ID_DIGITS && word.bytes().all(|b| b.is_ascii_digit()) {
            redacted.push_str("[steam id]");
        } else if word.len() == API_KEY_DIGITS && word.bytes().all(|b| b.is_ascii_hexdigit()) {
            redacted.push_str("[api key]");
        } else {
            redacted.push_str(word);
        }
        rest = after;
    }
    redacted.push_str(rest);

    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_ids_and_api_keys_never_reach_the_log() {
        assert_eq!(
            redact("profile 76561197960287930 not public"),
            "profile [steam id] not public"
        );
        assert_eq!(
            redact("GET /IPlayerService/?steamid=76561197960287930&key=Secret7 failed"),
            "GET /IPlayerService/?steamid=[steam id]&key=[api key] failed"
        );
        assert_eq!(
            redact("key 0123456789ABCDEF0123456789ABCDEF rejected"),
            "key [api key] rejected"
        );
    }

    #[test]
    fn the_rest_of_the_line_is_kept() {
        let line =
            "2026-11-02T10:00:00Z  WARN gamepalate_app::app: could not vote for 1145360: è già\n";

        assert_eq!(redact(line), line);
    }

    #[test]
    fn every_write_is_redacted() {
        let mut log = Redacted(Vec::new());

        let written = log.write(b"76561197960287930\n").unwrap();

        assert_eq!(written, 18);
        assert_eq!(log.0, b"[steam id]\n");
    }

    #[test]
    fn a_new_run_keeps_only_the_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");

        open_log(&logs).unwrap();
        fs::write(logs.join(FILE), "first").unwrap();
        open_log(&logs).unwrap();
        fs::write(logs.join(FILE), "second").unwrap();
        open_log(&logs).unwrap();

        assert_eq!(fs::read_to_string(logs.join(PREVIOUS)).unwrap(), "second");
        assert_eq!(fs::read_to_string(logs.join(FILE)).unwrap(), "");
    }
}
