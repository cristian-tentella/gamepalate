use std::path::Path;

use gamepalate_app::update::rebuild_catalog;

use crate::format::{date_time, integer};

pub fn run(data_dir: &Path) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let rebuilt = runtime.block_on(rebuild_catalog(data_dir))?;

    println!(
        "Catalog rebuilt from {}: {} games out of {} apps.",
        rebuilt.dir.display(),
        integer(rebuilt.games as u64),
        integer(rebuilt.apps as u64)
    );
    println!("Prices as of {}.", date_time(rebuilt.prices_at));
    Ok(())
}
