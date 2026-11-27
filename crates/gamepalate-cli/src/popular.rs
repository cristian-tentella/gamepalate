use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use gamepalate_core::popular::{popular_apps, popular_apps_text};
use gamepalate_store::catalog::load_catalog;

use crate::format::{date, integer};

#[derive(Args)]
pub struct PopularAppsArgs {
    /// File to write; by default popular-apps.txt in the current folder
    file: Option<PathBuf>,
}

pub fn run(args: &PopularAppsArgs, data_dir: &Path) -> anyhow::Result<()> {
    let file = args
        .file
        .clone()
        .unwrap_or_else(|| PathBuf::from("popular-apps.txt"));
    let count = write_popular_apps(data_dir, &file)?;
    println!(
        "Wrote {} popular apps to {}.",
        integer(count as u64),
        file.display()
    );
    Ok(())
}

fn write_popular_apps(data_dir: &Path, file: &Path) -> anyhow::Result<usize> {
    let catalog = load_catalog(&data_dir.join("catalog.sqlite"))?;
    let apps = popular_apps(&catalog);
    let text = popular_apps_text(&apps, &date(catalog.freshness.prices_updated_at));
    write_atomically(file, &text).with_context(|| format!("could not write {}", file.display()))?;
    Ok(apps.len())
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use gamepalate_core::{
        catalog::{Catalog, Freshness},
        currency::Currency,
        game::{Game, Price, Reviews},
        ids::AppId,
        popular::parse_popular_apps,
        region::Region,
        timestamp::Timestamp,
    };
    use gamepalate_store::catalog::write_catalog;

    use super::*;

    const PRICES_AT: i64 = 1_764_244_800_000;

    fn game(id: u32, total: u32, positive: u32) -> Game {
        Game {
            id: AppId(id),
            name: format!("Game {id}"),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews { total, positive },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn write_test_catalog(data_dir: &Path) {
        let catalog = Catalog::new(
            vec![
                game(10, 800, 700),
                game(20, 90_000, 85_000),
                game(30, 749, 749),
                game(40, 2_000, 1_000),
                game(50, 800, 800),
            ],
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(PRICES_AT),
                tags_updated_at: Timestamp(PRICES_AT),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        write_catalog(&data_dir.join("catalog.sqlite"), &catalog, &HashMap::new()).unwrap();
    }

    #[test]
    fn the_command_writes_the_popular_apps_of_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write_test_catalog(data.path());
        let file = out.path().join("popular-apps.txt");

        let count = write_popular_apps(data.path(), &file).unwrap();

        assert_eq!(count, 3);
        let text = std::fs::read_to_string(&file).unwrap();
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            format!(
                "# gamepalate popular apps · {} · 750 reviews · 72% positive · 3 apps",
                date(Timestamp(PRICES_AT))
            )
        );
        assert_eq!(
            parse_popular_apps(text.as_bytes()).unwrap(),
            [AppId(20), AppId(10), AppId(50)]
        );
        assert!(!out.path().join("popular-apps.txt.tmp").exists());
    }

    #[test]
    fn without_a_catalog_the_command_fails_and_writes_nothing() {
        let data = tempfile::tempdir().unwrap();
        let file = data.path().join("popular-apps.txt");

        assert!(write_popular_apps(data.path(), &file).is_err());
        assert!(!file.exists());
    }
}
