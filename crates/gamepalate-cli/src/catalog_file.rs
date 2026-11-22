use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::Args;
use gamepalate_app::{
    catalog_file::{
        ArchiveImport, ImportedCatalog, export_catalog, export_catalog_to, export_folder,
        import_catalog,
    },
    update::{archive_state, wall_clock},
};

use crate::format::{date_time, integer};

#[derive(Args)]
pub struct ExportArgs {
    /// Zip file to write; by default gamepalate-catalog-<date>.zip in the Downloads folder
    file: Option<PathBuf>,

    /// Put the archive of the updates in the zip too, so the next update of a friend in the same region is small
    #[arg(long)]
    with_archive: bool,
}

#[derive(Args)]
pub struct ImportArgs {
    /// A zip with catalog.sqlite in it, or a catalog.sqlite
    file: PathBuf,

    /// Import even if the archive has completed updates, and replace the archive with the one in the zip
    #[arg(long)]
    force: bool,
}

pub fn export(args: &ExportArgs, data_dir: &Path) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let file = match &args.file {
        Some(file) => {
            runtime.block_on(export_catalog_to(data_dir, file, args.with_archive))?;
            file.clone()
        }
        None => runtime.block_on(export_catalog(
            data_dir,
            &export_folder(data_dir),
            args.with_archive,
        ))?,
    };
    println!("Catalog exported to {}.", file.display());
    Ok(())
}

pub fn import(args: &ImportArgs, data_dir: &Path) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let imported = runtime.block_on(checked_import(args, data_dir))?;
    println!(
        "Catalog imported: {} games, prices of {} for {}.",
        integer(imported.games as u64),
        date_time(imported.prices_at),
        imported.region
    );
    if let Some(line) = archive_line(imported.archive) {
        println!("{line}");
    }
    Ok(())
}

fn archive_line(archive: ArchiveImport) -> Option<String> {
    match archive {
        ArchiveImport::Absent => None,
        ArchiveImport::Imported { updates: 1 } => Some("Archive imported: 1 update.".to_string()),
        ArchiveImport::Imported { updates } => Some(format!(
            "Archive imported: {} updates.",
            integer(updates as u64)
        )),
        ArchiveImport::Discarded(reason) => Some(format!("Archive not used: {reason}.")),
    }
}

async fn checked_import(args: &ImportArgs, data_dir: &Path) -> anyhow::Result<ImportedCatalog> {
    if !args.force && archive_state(data_dir, wall_clock())?.updated_at.is_some() {
        bail!(
            "the archive has completed updates, and the next update would rebuild the Catalog \
            from them: use --force to import anyway"
        );
    }
    Ok(import_catalog(data_dir, &args.file, args.force).await?)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamepalate_core::{region::Region, timestamp::Timestamp};
    use gamepalate_steam::archive::Archive;

    use super::*;

    #[tokio::test]
    async fn import_without_force_after_a_completed_update_touches_nothing() {
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

        let error = checked_import(&args, data.path()).await.unwrap_err();

        assert!(error.to_string().contains("--force"));
        let mut files: Vec<_> = fs::read_dir(data.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        files.sort();
        assert_eq!(files, ["archive"]);

        let forced = ImportArgs { file, force: true };
        assert!(
            checked_import(&forced, data.path())
                .await
                .is_err_and(|error| {
                    error
                        .to_string()
                        .starts_with("This isn't a gamepalate catalog")
                })
        );
    }

    #[test]
    fn the_archive_line_says_how_the_archive_went() {
        assert_eq!(archive_line(ArchiveImport::Absent), None);
        assert_eq!(
            archive_line(ArchiveImport::Imported { updates: 1 }).unwrap(),
            "Archive imported: 1 update."
        );
        assert_eq!(
            archive_line(ArchiveImport::Imported { updates: 79 }).unwrap(),
            "Archive imported: 79 updates."
        );
        assert_eq!(
            archive_line(ArchiveImport::Discarded("it's for another region")).unwrap(),
            "Archive not used: it's for another region."
        );
    }
}
