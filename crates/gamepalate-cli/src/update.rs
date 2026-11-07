use std::path::Path;

use gamepalate_app::{
    account::steam_api_key,
    update::{Progress, UpdateError, Updated, renew_catalog, update_catalog},
};
use gamepalate_core::config::Config;
use gamepalate_steam::transport::Transport;
use gamepalate_store::config::{ConfigError, load_config};

use crate::format::{date_time, integer};

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let config = match load_config(&data_dir.join("config.toml")) {
        Err(ConfigError::NotFound(_)) => Config::default(),
        config => config?,
    };
    let transport = Transport::new(steam_api_key())?;
    let runtime = tokio::runtime::Runtime::new()?;

    let updated = runtime
        .block_on(update_catalog(
            data_dir,
            &transport,
            &config,
            print_progress,
        ))
        .map_err(update_error)?;
    print_updated(&updated);
    if updated.full || updated.renewal.is_some() {
        return Ok(());
    }

    let renewed = runtime
        .block_on(renew_catalog(data_dir, &transport, &config, print_progress))
        .map_err(update_error)?;
    print_updated(&renewed);
    Ok(())
}

fn print_updated(updated: &Updated) {
    if let Some(error) = &updated.rotate_error {
        eprintln!("warn: could not remove the old updates from the archive: {error}");
    }

    println!(
        "Catalog updated: {} games out of {} apps (store data downloaded for {} apps).",
        integer(updated.games as u64),
        integer(updated.apps as u64),
        integer(updated.asked as u64)
    );
    println!("Prices as of {}.", date_time(updated.prices_at));
    if let Some(renewal) = &updated.renewal
        && renewal.still_expired > 0
    {
        println!(
            "{} apps are still expired: they will be renewed on the next update.",
            integer(renewal.still_expired as u64)
        );
    }
}

fn print_progress(progress: Progress) {
    match progress {
        Progress::Resumed {
            started_at,
            responses,
        } => println!(
            "Resuming the update started {}: {} responses already archived.",
            date_time(started_at),
            integer(responses as u64)
        ),
        Progress::Renewing { apps: 0, .. } => {}
        Progress::Renewing { apps, requests } => println!(
            "Renewing tags and reviews for {} apps ({} requests).",
            integer(apps as u64),
            integer(requests as u64)
        ),
        Progress::ReadingApps | Progress::StoreData { done: 0, .. } => {}
        Progress::StoreData { done, total } => eprint!("\rStore data: {done} of {total} requests"),
        Progress::Writing => eprintln!(),
    }
}

fn update_error(error: UpdateError) -> anyhow::Error {
    match error {
        UpdateError::Stopped {
            resume_before,
            source,
        } => anyhow::Error::new(source).context(format!(
            "the Catalog was not changed: run update again before {} to resume",
            date_time(resume_before)
        )),
        UpdateError::Locked => anyhow::anyhow!("Another update is running."),
        error => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_core::timestamp::Timestamp;
    use gamepalate_steam::transport::SteamError;

    use super::*;

    #[test]
    fn a_stopped_update_says_until_when_it_can_resume() {
        let error = update_error(UpdateError::Stopped {
            resume_before: Timestamp(1_793_638_800_000),
            source: SteamError::SteamDown {
                endpoint: "IStoreBrowseService/GetItems/v1".to_string(),
                reason: "timed out".to_string(),
                source: None,
            },
        });

        let text = format!("{error:#}");

        assert!(text.contains(&format!(
            "run update again before {} to resume",
            date_time(Timestamp(1_793_638_800_000))
        )));
        assert!(text.contains("timed out"));
    }

    #[test]
    fn a_locked_update_says_another_one_is_running() {
        let text = format!("{:#}", update_error(UpdateError::Locked));

        assert_eq!(text, "Another update is running.");
    }
}
