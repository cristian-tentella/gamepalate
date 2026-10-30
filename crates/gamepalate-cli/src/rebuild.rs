use std::path::Path;

use anyhow::Context;

use crate::{
    format::{date_time, integer},
    update::{archive, build_catalog, check_base},
};

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let chain = archive(data_dir)
        .chain()
        .context("could not read the archive")?;
    let Some((latest, earlier)) = chain.split_first() else {
        anyhow::bail!("the archive has no completed update: run update first");
    };
    check_base(&chain, latest.dir())?;

    let built = build_catalog(latest, latest.started_at(), earlier, latest.region())?;
    built.write(data_dir)?;

    println!(
        "Catalog rebuilt from {}: {} games out of {} apps.",
        latest.dir().display(),
        integer(built.catalog.games().len() as u64),
        integer(built.apps as u64)
    );
    println!("Prices as of {}.", date_time(latest.started_at()));
    Ok(())
}
