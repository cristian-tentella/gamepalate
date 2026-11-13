use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::Args;
use gamepalate_app::{
    catalog_file::{ImportedCatalog, export_catalog, export_catalog_to, import_catalog},
    update::{archive_state, wall_clock},
};

use crate::format::{date_time, integer};

#[derive(Args)]
pub struct ExportArgs {
    /// Zip file to write; by default gamepalate-catalog-<date>.zip in the Downloads folder
    file: Option<PathBuf>,
}

#[derive(Args)]
pub struct ImportArgs {
    /// A zip with catalog.sqlite in it, or a catalog.sqlite
    file: PathBuf,

    /// Import even if the archive has completed updates
    #[arg(long)]
    force: bool,
}

pub fn export(args: &ExportArgs, data_dir: &Path) -> anyhow::Result<()> {
    let file = match &args.file {
        Some(file) => {
            export_catalog_to(data_dir, file)?;
            file.clone()
        }
        None => {
            let into = dirs::download_dir().unwrap_or_else(|| data_dir.to_path_buf());
            export_catalog(data_dir, &into)?
        }
    };
    println!("Catalog exported to {}.", file.display());
    Ok(())
}

pub fn import(args: &ImportArgs, data_dir: &Path) -> anyhow::Result<()> {
    let imported = checked_import(args, data_dir)?;
    println!(
        "Catalog imported: {} games, prices of {} for {}.",
        integer(imported.games as u64),
        date_time(imported.prices_at),
        imported.region
    );
    Ok(())
}

fn checked_import(args: &ImportArgs, data_dir: &Path) -> anyhow::Result<ImportedCatalog> {
    if !args.force && archive_state(data_dir, wall_clock())?.updated_at.is_some() {
        bail!(
            "the archive has completed updates, and the next update would rebuild the Catalog \
            from them: use --force to import anyway"
        );
    }
    Ok(import_catalog(data_dir, &args.file)?)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_core::{region::Region, timestamp::Timestamp};
    use gamepalate_steam::archive::Archive;

    use super::*;

    #[test]
    fn import_without_force_after_a_completed_update_touches_nothing() {
        let data = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        Archive::new(data.path().join("archive"))
            .start_update(Timestamp(0), 0, Region::ITALY, false)
            .unwrap()
            .complete(Timestamp(0))
            .unwrap();
        let file = source.path().join("catalog.sqlite");
        fs::write(&file, "not a catalog").unwrap();
        let args = ImportArgs {
            file: file.clone(),
            force: false,
        };

        let error = checked_import(&args, data.path()).unwrap_err();

        assert!(error.to_string().contains("--force"));
        let mut files: Vec<_> = fs::read_dir(data.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        files.sort();
        assert_eq!(files, ["archive"]);

        let forced = ImportArgs { file, force: true };
        assert!(checked_import(&forced, data.path()).is_err_and(|error| {
            error
                .to_string()
                .starts_with("This isn't a gamepalate catalog")
        }));
    }
}
