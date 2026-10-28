use std::path::Path;

use anyhow::Context;

use crate::{
    format::{date_time, integer},
    update::{archive, build_catalog},
};

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let update = archive(data_dir)
        .latest_completed()
        .context("could not read the archive")?
        .context("the archive has no completed update: run update first")?;

    let built = build_catalog(&update, update.started_at())?;
    built.write(data_dir)?;

    println!(
        "Catalog rebuilt from {}: {} games out of {} apps.",
        update.dir().display(),
        integer(built.catalog.games().len() as u64),
        integer(built.apps as u64)
    );
    println!("Prices as of {}.", date_time(update.started_at()));
    Ok(())
}
