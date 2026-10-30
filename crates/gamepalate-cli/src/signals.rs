use std::path::Path;

use anyhow::Context;
use gamepalate_app::{
    account::steam_api_key,
    signals::{OffCatalogInputs, SignalsWarning, off_catalog, read_signals, save_signals},
    update::wall_clock,
};
use gamepalate_core::config::Config;
use gamepalate_steam::{
    signals::{FailedList, OptionalList},
    transport::Transport,
};
use gamepalate_store::{
    config::load_config,
    data_files::{DataFiles, load_data_files, load_data_files_with},
};

use crate::format::date_time;

pub fn load(data_dir: &Path, offline: bool) -> anyhow::Result<DataFiles> {
    Ok(load_for(data_dir, offline, |config| *config)?.0)
}

pub fn load_for(
    data_dir: &Path,
    offline: bool,
    adjust: impl FnOnce(&Config) -> Config,
) -> anyhow::Result<(DataFiles, Config)> {
    if offline {
        let files = load_data_files(data_dir)?;
        eprintln!("Signals read on {}.", date_time(files.signals.read_at));
        let config = adjust(&files.config);
        return Ok((files, config));
    }

    let config = load_config(&data_dir.join("config.toml"))?;
    let steam_id = config
        .steam_id
        .context("steam_id is missing from the config")?;
    let transport = Transport::new(steam_api_key())?;
    let runtime = tokio::runtime::Runtime::new()?;
    let now = wall_clock();

    let read = runtime.block_on(read_signals(data_dir, &transport, steam_id, now))?;
    save_signals(data_dir, &read)?;
    for warning in &read.warnings {
        eprintln!("warn: {}", warning_text(warning));
    }

    let mut files = load_data_files_with(data_dir, config, read.signals())?;
    let adjusted = adjust(&files.config);
    files.off_catalog = runtime.block_on(off_catalog(
        data_dir,
        &transport,
        OffCatalogInputs {
            signals: &files.signals,
            votes: &files.votes,
            families: &files.families,
            catalog: &files.catalog,
            configs: &[&files.config, &adjusted],
        },
        now,
    ))?;
    Ok((files, adjusted))
}

fn warning_text(warning: &SignalsWarning) -> String {
    match warning {
        SignalsWarning::SteamDown { read_at } => format!(
            "Steam is not responding: using the signals read on {}.",
            date_time(*read_at)
        ),
        SignalsWarning::ListFailed(failed) => failed_warning(failed),
    }
}

fn failed_warning(failed: &FailedList) -> String {
    let (name, one, it) = match failed.list {
        OptionalList::Wishlist => ("Wishlist", "the one", "it"),
        OptionalList::Followed => ("Followed games", "the ones", "them"),
    };
    match failed.saved_from {
        Some(read_at) => format!(
            "{name} unavailable: using {one} read on {}.",
            date_time(read_at)
        ),
        None => format!("{name} unavailable: going on without {it}."),
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_core::timestamp::Timestamp;

    use super::*;

    const READ_AT: Timestamp = Timestamp(1_793_000_000_000);

    #[test]
    fn steam_down_says_when_the_signals_were_read() {
        assert_eq!(
            warning_text(&SignalsWarning::SteamDown { read_at: READ_AT }),
            format!(
                "Steam is not responding: using the signals read on {}.",
                date_time(READ_AT)
            )
        );
    }

    #[test]
    fn a_failed_wishlist_says_which_one_is_used() {
        let failed = |saved_from| {
            warning_text(&SignalsWarning::ListFailed(FailedList {
                list: OptionalList::Wishlist,
                saved_from,
            }))
        };

        assert_eq!(failed(None), "Wishlist unavailable: going on without it.");
        assert_eq!(
            failed(Some(READ_AT)),
            format!(
                "Wishlist unavailable: using the one read on {}.",
                date_time(READ_AT)
            )
        );
    }

    #[test]
    fn a_failed_followed_list_is_plural() {
        let failed = FailedList {
            list: OptionalList::Followed,
            saved_from: None,
        };

        assert_eq!(
            failed_warning(&failed),
            "Followed games unavailable: going on without them."
        );
    }
}
