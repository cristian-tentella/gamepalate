use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use gamepalate_core::{
    catalog::tag_frequencies,
    starter::{starter_apps, starter_catalog_text},
};
use gamepalate_store::catalog::load_catalog;

use crate::format::{date, integer};

#[derive(Args)]
pub struct StarterCatalogArgs {
    /// File to write; by default starter-catalog.txt in the current folder
    file: Option<PathBuf>,
}

pub fn run(args: &StarterCatalogArgs, data_dir: &Path) -> anyhow::Result<()> {
    let file = args
        .file
        .clone()
        .unwrap_or_else(|| PathBuf::from("starter-catalog.txt"));
    let (apps, tags) = write_starter_catalog(data_dir, &file)?;
    println!(
        "Wrote {} popular apps and {} tags to {}.",
        integer(apps as u64),
        integer(tags as u64),
        file.display()
    );
    Ok(())
}

fn write_starter_catalog(data_dir: &Path, file: &Path) -> anyhow::Result<(usize, usize)> {
    let catalog = load_catalog(&data_dir.join("catalog.sqlite"))?;
    let apps = starter_apps(&catalog);
    let tags = tag_frequencies(catalog.games());
    let text = starter_catalog_text(&apps, &tags, &date(catalog.freshness.prices_updated_at));
    write_atomically(file, &text).with_context(|| format!("could not write {}", file.display()))?;
    Ok((apps.len(), tags.tags.len()))
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
        catalog::{Catalog, Freshness, TagFrequencies, compute_idf, idf_from},
        currency::Currency,
        game::{Game, GameTag, Price, Reviews},
        ids::{AppId, TagId},
        region::Region,
        starter::{StarterCatalog, parse_starter_catalog},
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
            tags: vec![
                GameTag {
                    id: TagId(19),
                    weight: if id == 30 { 0.0 } else { 0.5 },
                },
                GameTag {
                    id: TagId(492),
                    weight: if id == 984 { 0.5 } else { 0.0 },
                },
            ],
            developers: Vec::new(),
            franchises: Vec::new(),
        }
    }

    fn write_test_catalog(data_dir: &Path) -> Catalog {
        let games = vec![
            game(10, 800, 700),
            game(20, 90_000, 85_000),
            game(30, 749, 749),
            game(40, 2_000, 1_000),
            game(50, 800, 800),
            game(984, 10, 10),
        ];
        let idf = compute_idf(&games);
        let catalog = Catalog::new(
            games,
            BTreeMap::new(),
            idf,
            Freshness {
                prices_updated_at: Timestamp(PRICES_AT),
                tags_updated_at: Timestamp(PRICES_AT),
                region: Region::ITALY,
            },
            Currency::italian(),
        );
        write_catalog(&data_dir.join("catalog.sqlite"), &catalog, &HashMap::new()).unwrap();
        catalog
    }

    #[test]
    fn the_command_writes_the_popular_apps_and_the_tags_of_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let catalog = write_test_catalog(data.path());
        let file = out.path().join("starter-catalog.txt");

        let counts = write_starter_catalog(data.path(), &file).unwrap();

        assert_eq!(counts, (3, 2));
        let text = std::fs::read_to_string(&file).unwrap();
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            format!(
                "# gamepalate starter catalog · {} · 750 reviews · 72% positive · 3 apps · 2 tags",
                date(Timestamp(PRICES_AT))
            )
        );
        let starter = parse_starter_catalog(text.as_bytes()).unwrap();
        let tags = TagFrequencies {
            games: 6,
            tags: [(TagId(19), 5), (TagId(492), 1)].into(),
        };
        assert_eq!(
            starter,
            StarterCatalog {
                apps: vec![AppId(20), AppId(10), AppId(50)],
                tags: Some(tags.clone()),
            }
        );
        assert_eq!(idf_from(&tags), catalog.idf);
        assert!(!out.path().join("starter-catalog.txt.tmp").exists());
    }

    #[test]
    fn without_a_catalog_the_command_fails_and_writes_nothing() {
        let data = tempfile::tempdir().unwrap();
        let file = data.path().join("starter-catalog.txt");

        assert!(write_starter_catalog(data.path(), &file).is_err());
        assert!(!file.exists());
    }
}
