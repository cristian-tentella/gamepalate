use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use gamepalate_core::{region::Region, timestamp::Timestamp};
use gamepalate_steam::{
    archive::{Archive, ArchivedUpdate, is_update_file, is_update_folder},
    chain::{ChainError, chain},
};
use gamepalate_store::catalog::{CatalogError, read_catalog_file, read_catalog_freshness};
use jiff::tz::TimeZone;
use zip::{CompressionMethod, ZipArchive, ZipWriter, result::ZipError, write::SimpleFileOptions};

use crate::{
    lock::{LockError, UpdateLock},
    update::archive,
};

const CATALOG: &str = "catalog.sqlite";
const CACHE: &str = "catalog.bin";
const IMPORT: &str = "import.sqlite";
const ARCHIVE: &str = "archive";
const IMPORT_ARCHIVE: &str = "import-archive";
const REPLACED_ARCHIVE: &str = "replaced-archive";
const ZIP_ARCHIVE: &str = "archive/";
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";
const MAX_BYTES: u64 = 1 << 30;
const MAX_ARCHIVE_BYTES: u64 = 2 << 30;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImportedCatalog {
    pub games: usize,
    pub prices_at: Timestamp,
    pub region: Region,
    pub archive: ArchiveImport,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArchiveImport {
    Absent,
    Imported { updates: usize },
    Discarded(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogFileError {
    #[error("This file has no catalog in it")]
    NoCatalog,

    #[error("This isn't a gamepalate catalog, or it comes from an older version")]
    NotACatalog(#[source] Option<CatalogError>),

    #[error("The catalog in this file is larger than 1 GB")]
    TooLarge,

    #[error("This catalog has no complete archive to export")]
    NoArchive,

    #[error("An update is running. Try again when it ends")]
    Locked,

    #[error("could not lock the data folder")]
    Lock(#[source] io::Error),

    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error("could not read the archive")]
    Archive(#[source] ChainError),

    #[error("could not read the zip file")]
    Zip(#[source] ZipError),

    #[error("could not read {}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write {}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn export_folder(data_dir: &Path) -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| data_dir.to_path_buf())
}

pub async fn export_catalog(
    data_dir: &Path,
    into_dir: &Path,
    with_archive: bool,
) -> Result<PathBuf, CatalogFileError> {
    let _lock = lock(data_dir)?;
    let catalog = data_dir.join(CATALOG);
    let freshness = read_catalog_freshness(&catalog)?;
    let chain = exported_chain(data_dir, freshness.region, with_archive).await?;
    let file = free_path(
        into_dir,
        &file_name(freshness.prices_updated_at, with_archive),
    );
    write_zip(&catalog, &chain, &file)?;
    Ok(file)
}

pub async fn export_catalog_to(
    data_dir: &Path,
    file: &Path,
    with_archive: bool,
) -> Result<(), CatalogFileError> {
    let _lock = lock(data_dir)?;
    let catalog = data_dir.join(CATALOG);
    let freshness = read_catalog_freshness(&catalog)?;
    let chain = exported_chain(data_dir, freshness.region, with_archive).await?;
    write_zip(&catalog, &chain, file)
}

pub async fn import_catalog(
    data_dir: &Path,
    file: &Path,
    replace_archive: bool,
) -> Result<ImportedCatalog, CatalogFileError> {
    let _lock = lock(data_dir)?;
    let imported = data_dir.join(IMPORT);
    let result = place_catalog(file, &imported, data_dir);
    if result.is_err() {
        let _ = fs::remove_file(&imported);
    }
    let mut catalog = result?;
    catalog.archive = place_archive(file, data_dir, catalog.region, replace_archive).await;
    Ok(catalog)
}

fn lock(data_dir: &Path) -> Result<UpdateLock, CatalogFileError> {
    UpdateLock::take(data_dir).map_err(|error| match error {
        LockError::Taken => CatalogFileError::Locked,
        LockError::Io(source) => CatalogFileError::Lock(source),
    })
}

async fn exported_chain(
    data_dir: &Path,
    region: Region,
    with_archive: bool,
) -> Result<Vec<ArchivedUpdate>, CatalogFileError> {
    if !with_archive {
        return Ok(Vec::new());
    }
    let chain = match chain(&archive(data_dir)).await {
        Err(ChainError::Uncovered { .. }) => Vec::new(),
        chain => chain.map_err(CatalogFileError::Archive)?,
    };
    match chain.first() {
        Some(latest) if latest.region() == region => Ok(chain),
        _ => Err(CatalogFileError::NoArchive),
    }
}

fn file_name(prices_at: Timestamp, with_archive: bool) -> String {
    let date = jiff::Timestamp::from_millisecond(prices_at.0)
        .map(|time| time.to_zoned(TimeZone::system()).date().to_string())
        .unwrap_or_default();
    let archive = if with_archive { "-with-archive" } else { "" };
    format!("gamepalate-catalog-{date}{archive}")
}

fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.zip"));
    let mut copy = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({copy}).zip"));
        copy += 1;
    }
    path
}

fn write_zip(
    catalog: &Path,
    chain: &[ArchivedUpdate],
    file: &Path,
) -> Result<(), CatalogFileError> {
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let result = zip_into(catalog, chain, &temporary).and_then(|()| {
        fs::rename(&temporary, file).map_err(|source| CatalogFileError::Write {
            path: file.to_path_buf(),
            source,
        })
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn zip_into(
    catalog: &Path,
    chain: &[ArchivedUpdate],
    temporary: &Path,
) -> Result<(), CatalogFileError> {
    let write_error = |source| CatalogFileError::Write {
        path: temporary.to_path_buf(),
        source,
    };
    let read_error = |path: &Path| {
        let path = path.to_path_buf();
        move |source| CatalogFileError::Read { path, source }
    };
    let mut source = File::open(catalog).map_err(read_error(catalog))?;

    let mut zip = ZipWriter::new(File::create(temporary).map_err(write_error)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(CATALOG, options)
        .map_err(|error| write_error(error.into()))?;
    io::copy(&mut source, &mut zip).map_err(write_error)?;

    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for update in chain {
        let folder = update
            .dir()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        for name in update_files(update.dir()).map_err(read_error(update.dir()))? {
            let path = update.dir().join(&name);
            let mut source = File::open(&path).map_err(read_error(&path))?;
            zip.start_file(format!("{ZIP_ARCHIVE}{folder}/{name}"), stored)
                .map_err(|error| write_error(error.into()))?;
            io::copy(&mut source, &mut zip).map_err(write_error)?;
        }
    }

    zip.finish()
        .map_err(|error| write_error(error.into()))?
        .sync_all()
        .map_err(write_error)
}

fn update_files(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        if let Ok(name) = entry?.file_name().into_string()
            && is_update_file(&name)
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn place_catalog(
    file: &Path,
    imported: &Path,
    data_dir: &Path,
) -> Result<ImportedCatalog, CatalogFileError> {
    copy_catalog(file, imported)?;

    let catalog =
        read_catalog_file(imported).map_err(|error| CatalogFileError::NotACatalog(Some(error)))?;
    if catalog.games().is_empty() {
        return Err(CatalogFileError::NotACatalog(None));
    }

    let target = data_dir.join(CATALOG);
    fs::rename(imported, &target).map_err(|source| CatalogFileError::Write {
        path: target.clone(),
        source,
    })?;
    let cache = data_dir.join(CACHE);
    match fs::remove_file(&cache) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => {
            return Err(CatalogFileError::Write {
                path: cache,
                source,
            });
        }
        _ => {}
    }

    Ok(ImportedCatalog {
        games: catalog.games().len(),
        prices_at: catalog.freshness.prices_updated_at,
        region: catalog.freshness.region,
        archive: ArchiveImport::Absent,
    })
}

fn copy_catalog(file: &Path, imported: &Path) -> Result<(), CatalogFileError> {
    if let Some(mut archive) = open_zip(file)? {
        let index = (0..archive.len())
            .find(|&index| {
                matches!(
                    archive.name_for_index(index),
                    Some(Ok(name)) if name.rsplit(['/', '\\']).next() == Some(CATALOG)
                )
            })
            .ok_or(CatalogFileError::NoCatalog)?;
        let entry = archive.by_index(index).map_err(CatalogFileError::Zip)?;
        if entry.size() > MAX_BYTES {
            return Err(CatalogFileError::TooLarge);
        }
        write_limited(entry, imported)
    } else {
        let source = File::open(file).map_err(|source| CatalogFileError::Read {
            path: file.to_path_buf(),
            source,
        })?;
        write_limited(source, imported)
    }
}

fn open_zip(file: &Path) -> Result<Option<ZipArchive<File>>, CatalogFileError> {
    let read_error = |source| CatalogFileError::Read {
        path: file.to_path_buf(),
        source,
    };
    let mut source = File::open(file).map_err(read_error)?;
    let mut magic = [0; 4];
    match source.read_exact(&mut magic) {
        Ok(()) if &magic == ZIP_MAGIC => ZipArchive::new(source)
            .map(Some)
            .map_err(CatalogFileError::Zip),
        Ok(()) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(read_error(error)),
    }
}

fn write_limited(source: impl Read, imported: &Path) -> Result<(), CatalogFileError> {
    let write_error = |source| CatalogFileError::Write {
        path: imported.to_path_buf(),
        source,
    };
    let mut target = File::create(imported).map_err(write_error)?;
    let written = io::copy(&mut source.take(MAX_BYTES + 1), &mut target).map_err(write_error)?;
    if written > MAX_BYTES {
        return Err(CatalogFileError::TooLarge);
    }
    target.flush().map_err(write_error)
}

async fn place_archive(
    file: &Path,
    data_dir: &Path,
    region: Region,
    replace: bool,
) -> ArchiveImport {
    let staging = data_dir.join(IMPORT_ARCHIVE);
    let _ = fs::remove_dir_all(&staging);
    let result = import_archive(file, data_dir, &staging, region, replace).await;
    let _ = fs::remove_dir_all(&staging);
    match result {
        Ok(None) => ArchiveImport::Absent,
        Ok(Some(updates)) => ArchiveImport::Imported { updates },
        Err(reason) => ArchiveImport::Discarded(reason),
    }
}

fn because<E: std::fmt::Display>(reason: &'static str) -> impl FnOnce(E) -> &'static str {
    move |error| {
        tracing::warn!("archive not imported, {reason}: {error}");
        reason
    }
}

async fn import_archive(
    file: &Path,
    data_dir: &Path,
    staging: &Path,
    region: Region,
    replace: bool,
) -> Result<Option<usize>, &'static str> {
    const UNREADABLE: &str = "it couldn't be read";
    const UNWRITABLE: &str = "it couldn't be written";

    let Some(mut zip) = open_zip(file).map_err(because(UNREADABLE))? else {
        return Ok(None);
    };
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let Some(Ok(name)) = zip.name_for_index(index) else {
            continue;
        };
        let Some(path) = name.strip_prefix(ZIP_ARCHIVE) else {
            continue;
        };
        match path.split_once('/') {
            Some((folder, name)) if is_update_folder(folder) && is_update_file(name) => {
                entries.push((index, folder.to_string(), name.to_string()));
            }
            _ => return Err("it has a file that isn't part of an archive"),
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }

    let only_unfinished = archive(data_dir)
        .completed()
        .is_ok_and(|completed| completed.is_empty());
    if !only_unfinished && !replace {
        return Err("there is already an archive with completed updates");
    }

    let mut left = MAX_ARCHIVE_BYTES;
    for (index, folder, name) in &entries {
        let entry = zip.by_index(*index).map_err(because(UNREADABLE))?;
        let dir = staging.join(folder);
        fs::create_dir_all(&dir).map_err(because(UNWRITABLE))?;
        let mut target = File::create(dir.join(name)).map_err(because(UNWRITABLE))?;
        let written =
            io::copy(&mut entry.take(left + 1), &mut target).map_err(because(UNREADABLE))?;
        if written > left {
            return Err("it's larger than 2 GB");
        }
        left -= written;
        target.sync_all().map_err(because(UNWRITABLE))?;
    }

    let staged = Archive::new(staging.to_path_buf());
    let completed = staged.completed().map_err(because(UNREADABLE))?;
    let folders: HashSet<&str> = entries
        .iter()
        .map(|(_, folder, _)| folder.as_str())
        .collect();
    if completed.len() != folders.len() {
        return Err("it has updates that didn't complete");
    }
    if completed.iter().any(|update| update.region() != region) {
        return Err("it's for another region");
    }
    match chain(&staged).await {
        Ok(chain) if !chain.is_empty() => {}
        Ok(_) | Err(ChainError::Uncovered { .. }) => return Err("some games are missing from it"),
        Err(error) => return Err(because(UNREADABLE)(error)),
    }

    swap_archive(data_dir, staging).map_err(because(UNWRITABLE))?;
    Ok(Some(completed.len()))
}

fn swap_archive(data_dir: &Path, staging: &Path) -> io::Result<()> {
    let target = data_dir.join(ARCHIVE);
    let replaced = data_dir.join(REPLACED_ARCHIVE);
    let _ = fs::remove_dir_all(&replaced);
    let had_one = match fs::rename(&target, &replaced) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if let Err(error) = fs::rename(staging, &target) {
        if had_one {
            let _ = fs::rename(&replaced, &target);
        }
        return Err(error);
    }
    let _ = fs::remove_dir_all(&replaced);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use gamepalate_core::{
        catalog::{Catalog, Freshness},
        currency::Currency,
        game::{Game, Price, Reviews},
        ids::AppId,
    };
    use std::sync::Mutex;

    use gamepalate_steam::{
        apps::get_app_list,
        items::get_items,
        transport::{Get, SteamError},
    };
    use gamepalate_store::catalog::{load_catalog, write_catalog};
    use serde_json::{Value, json};

    use super::*;
    use crate::update::archive_state;

    const PRICES_AT: i64 = 1_763_000_000_000;

    fn catalog(names: &[&str], prices_at: i64) -> Catalog {
        let games = names
            .iter()
            .zip(1..)
            .map(|(name, id)| Game {
                id: AppId(id),
                name: (*name).to_string(),
                price: Price {
                    original_cents: 999,
                    discount: None,
                },
                reviews: Reviews {
                    total: 10,
                    positive: 9,
                },
                is_early_access: false,
                tags: Vec::new(),
                developers: Vec::new(),
                franchises: Vec::new(),
            })
            .collect();
        Catalog::new(
            games,
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(prices_at),
                tags_updated_at: Timestamp(prices_at),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn write(path: &Path, catalog: &Catalog) {
        write_catalog(path, catalog, &HashMap::new()).unwrap();
    }

    fn names(catalog: &Catalog) -> Vec<String> {
        catalog
            .games()
            .iter()
            .map(|game| game.name.clone())
            .collect()
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        files.sort();
        files
    }

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[tokio::test]
    async fn an_exported_catalog_imports_back_with_the_same_games() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(
            &from.path().join(CATALOG),
            &catalog(&["Alpha", "Beta"], PRICES_AT),
        );

        let zip = export_catalog(from.path(), downloads.path(), false)
            .await
            .unwrap();
        let imported = import_catalog(to.path(), &zip, false).await.unwrap();

        assert_eq!(
            imported,
            ImportedCatalog {
                games: 2,
                prices_at: Timestamp(PRICES_AT),
                region: Region::ITALY,
                archive: ArchiveImport::Absent,
            }
        );
        let catalog = load_catalog(&to.path().join(CATALOG)).unwrap();
        assert_eq!(names(&catalog), ["Alpha", "Beta"]);
        assert_eq!(files(downloads.path()), [zip_name(PRICES_AT, "")]);
    }

    fn zip_name(prices_at: i64, copy: &str) -> String {
        let date = jiff::Timestamp::from_millisecond(prices_at)
            .unwrap()
            .to_zoned(TimeZone::system())
            .strftime("%Y-%m-%d");
        format!("gamepalate-catalog-{date}{copy}.zip")
    }

    #[tokio::test]
    async fn the_file_name_has_the_date_of_the_prices_and_a_copy_number() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        let first = export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();
        let second = export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();
        let third = export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();

        assert_eq!(
            [first, second, third].map(|path| path.file_name().unwrap().to_owned()),
            [
                zip_name(PRICES_AT, ""),
                zip_name(PRICES_AT, " (2)"),
                zip_name(PRICES_AT, " (3)"),
            ]
            .map(std::ffi::OsString::from)
        );
    }

    #[tokio::test]
    async fn the_zip_holds_only_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        let zip = export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();
        let mut archive = ZipArchive::new(File::open(zip).unwrap()).unwrap();

        assert_eq!(archive.len(), 1);
        let entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name().unwrap(), CATALOG);
        assert_eq!(entry.compression(), CompressionMethod::Deflated);
    }

    #[tokio::test]
    async fn export_to_a_named_file() {
        let data = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let file = target.path().join("friend.zip");
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        export_catalog_to(data.path(), &file, false).await.unwrap();

        assert_eq!(files(target.path()), ["friend.zip"]);
    }

    #[tokio::test]
    async fn a_loose_catalog_imports_into_a_missing_data_folder() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("gamepalate");
        let loose = source.path().join("catalog.sqlite");
        write(&loose, &catalog(&["Alpha"], PRICES_AT));

        let imported = import_catalog(&data, &loose, false).await.unwrap();

        assert_eq!(imported.games, 1);
        assert_eq!(files(&data), ["catalog.sqlite", "update.lock"]);
    }

    #[tokio::test]
    async fn a_catalog_in_a_zip_subfolder_imports() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let loose = source.path().join("loose.sqlite");
        write(&loose, &catalog(&["Alpha"], PRICES_AT));
        let zip = source.path().join("by-hand.zip");
        zip_with(
            &zip,
            &[
                ("readme.txt", b"hi"),
                ("gamepalate/catalog.sqlite", &fs::read(&loose).unwrap()),
            ],
        );

        assert_eq!(
            import_catalog(data.path(), &zip, false)
                .await
                .unwrap()
                .games,
            1
        );
    }

    #[tokio::test]
    async fn a_wrong_file_leaves_the_folder_as_it_was() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Old"], PRICES_AT));
        load_catalog(&data.path().join(CATALOG)).unwrap();
        let before = fs::read(data.path().join(CATALOG)).unwrap();

        let no_catalog = source.path().join("no-catalog.zip");
        zip_with(&no_catalog, &[("readme.txt", b"hi")]);
        let text = source.path().join("notes.txt");
        fs::write(&text, "not a catalog").unwrap();
        let empty_sqlite = source.path().join("empty.sqlite");
        write(&empty_sqlite, &catalog(&[], PRICES_AT));
        let empty_file = source.path().join("empty");
        fs::write(&empty_file, "").unwrap();

        assert!(matches!(
            import_catalog(data.path(), &no_catalog, false).await,
            Err(CatalogFileError::NoCatalog)
        ));
        for wrong in [&text, &empty_sqlite, &empty_file] {
            assert!(matches!(
                import_catalog(data.path(), wrong, false).await,
                Err(CatalogFileError::NotACatalog(_))
            ));
        }
        assert!(matches!(
            import_catalog(data.path(), &source.path().join("missing.zip"), false).await,
            Err(CatalogFileError::Read { .. })
        ));

        assert_eq!(
            files(data.path()),
            ["catalog.bin", "catalog.sqlite", "update.lock"]
        );
        assert_eq!(fs::read(data.path().join(CATALOG)).unwrap(), before);
    }

    #[tokio::test]
    async fn an_import_removes_the_cache_even_when_the_file_is_older() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let sqlite = data.path().join(CATALOG);
        write(&sqlite, &catalog(&["Old"], PRICES_AT));
        load_catalog(&sqlite).unwrap();
        assert!(data.path().join(CACHE).exists());

        let loose = source.path().join("catalog.sqlite");
        write(&loose, &catalog(&["New"], PRICES_AT - 1));
        File::options()
            .write(true)
            .open(&loose)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        import_catalog(data.path(), &loose, false).await.unwrap();

        assert!(!data.path().join(CACHE).exists());
        assert_eq!(names(&load_catalog(&sqlite).unwrap()), ["New"]);
    }

    #[tokio::test]
    async fn export_and_import_wait_for_a_running_update() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let sqlite = data.path().join(CATALOG);
        write(&sqlite, &catalog(&["Alpha"], PRICES_AT));
        let held = UpdateLock::take(data.path()).unwrap();

        assert!(matches!(
            export_catalog(data.path(), downloads.path(), false).await,
            Err(CatalogFileError::Locked)
        ));
        assert!(matches!(
            import_catalog(data.path(), &sqlite, false).await,
            Err(CatalogFileError::Locked)
        ));
        assert!(files(downloads.path()).is_empty());

        drop(held);
        export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_imported_catalog_has_no_completed_update() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let loose = source.path().join("catalog.sqlite");
        write(&loose, &catalog(&["Alpha"], PRICES_AT));

        import_catalog(data.path(), &loose, false).await.unwrap();
        let state = archive_state(data.path(), Timestamp(PRICES_AT)).unwrap();

        assert_eq!(state.updated_at, None);
        Archive::new(data.path().join("archive"))
            .start_update(Timestamp(PRICES_AT), 0, Region::ITALY, false)
            .unwrap()
            .complete(Timestamp(PRICES_AT))
            .unwrap();
        let state = archive_state(data.path(), Timestamp(PRICES_AT)).unwrap();
        assert_eq!(state.updated_at, Some(Timestamp(PRICES_AT)));
    }

    const KEY: &str = "0123456789ABCDEF0123456789ABCDEF";
    const HOUR: i64 = 3_600_000;
    const APP_LIST: &str = "IStoreService/GetAppList/v1";
    const ITEMS: &str = "IStoreBrowseService/GetItems/v1";

    struct FakeSteam {
        listed: Vec<u32>,
        keyed: Mutex<Vec<String>>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            needs_key: bool,
        ) -> Result<Value, SteamError> {
            if needs_key {
                self.keyed
                    .lock()
                    .unwrap()
                    .push(format!("{endpoint}?key={KEY}"));
            }
            match endpoint {
                APP_LIST => Ok(json!({ "response": { "apps": self
                    .listed
                    .iter()
                    .map(|appid| json!({
                        "appid": appid,
                        "last_modified": 1,
                        "price_change_number": 1,
                    }))
                    .collect::<Vec<_>>() } })),
                ITEMS => Ok(json!({ "response": {} })),
                other => panic!("unexpected request to {other}"),
            }
        }
    }

    async fn archived_update(
        data_dir: &Path,
        hours: i64,
        region: Region,
        listed: &[u32],
        asked: &[u32],
    ) -> usize {
        let started_at = Timestamp(PRICES_AT + hours * HOUR);
        let update = archive(data_dir)
            .start_update(started_at, 0, region, false)
            .unwrap();
        let steam = FakeSteam {
            listed: listed.to_vec(),
            keyed: Mutex::new(Vec::new()),
        };
        let asked: Vec<AppId> = asked.iter().copied().map(AppId).collect();
        {
            let archiving = update.archiving(&steam);
            get_app_list(&archiving).await.unwrap();
            get_items(&archiving, &asked, "english", region.as_str())
                .await
                .unwrap();
        }
        update.complete(started_at).unwrap();
        steam.keyed.lock().unwrap().len()
    }

    async fn exporter(region: Region) -> tempfile::TempDir {
        let data = tempfile::tempdir().unwrap();
        write(
            &data.path().join(CATALOG),
            &catalog(&["Alpha", "Beta"], PRICES_AT),
        );
        archived_update(data.path(), -2, region, &[1, 2], &[1, 2]).await;
        archived_update(data.path(), -1, region, &[1, 2, 3], &[3]).await;
        data
    }

    fn zip_entries(file: &Path) -> Vec<(String, CompressionMethod)> {
        let mut zip = ZipArchive::new(File::open(file).unwrap()).unwrap();
        (0..zip.len())
            .map(|index| {
                let entry = zip.by_index(index).unwrap();
                (entry.name().unwrap().to_string(), entry.compression())
            })
            .collect()
    }

    fn assert_only_the_catalog(data_dir: &Path, imported: &ImportedCatalog) {
        assert!(matches!(imported.archive, ArchiveImport::Discarded(_)));
        assert_eq!(imported.games, 2);
        assert_eq!(
            names(&load_catalog(&data_dir.join(CATALOG)).unwrap()),
            ["Alpha", "Beta"]
        );
        assert!(!data_dir.join(IMPORT_ARCHIVE).exists());
    }

    #[tokio::test]
    async fn an_export_with_the_archive_imports_back_with_its_chain() {
        let from = exporter(Region::ITALY).await;
        let to = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();

        let zip = export_catalog(from.path(), downloads.path(), true)
            .await
            .unwrap();
        let imported = import_catalog(to.path(), &zip, false).await.unwrap();

        assert_eq!(
            zip.file_name().unwrap().to_string_lossy(),
            zip_name(PRICES_AT, "").replace(".zip", "-with-archive.zip")
        );
        let entries = zip_entries(&zip);
        assert_eq!(
            entries[0],
            (CATALOG.to_string(), CompressionMethod::Deflated)
        );
        assert!(entries[1..].iter().all(|(name, compression)| {
            (name.starts_with("archive/000001/") || name.starts_with("archive/000002/"))
                && *compression == CompressionMethod::Stored
        }));
        assert_eq!(entries.len(), 1 + 2 * 3);

        assert_eq!(imported.archive, ArchiveImport::Imported { updates: 2 });
        assert_eq!(
            names(&load_catalog(&to.path().join(CATALOG)).unwrap()),
            ["Alpha", "Beta"]
        );
        assert_eq!(chain(&archive(to.path())).await.unwrap().len(), 2);
        assert_eq!(
            files(to.path()),
            ["archive", "catalog.bin", "catalog.sqlite", "update.lock"]
        );
        let now = Timestamp(PRICES_AT + HOUR);
        let state = archive_state(to.path(), now).unwrap();
        assert_eq!(state.updated_at, Some(Timestamp(PRICES_AT - HOUR)));
        let next = archive(to.path())
            .start_update(now, 0, imported.region, false)
            .unwrap();
        assert!(!next.full());
    }

    #[tokio::test]
    async fn no_file_of_the_exported_archive_holds_the_key() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&from.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));
        let keyed = archived_update(from.path(), 0, Region::ITALY, &[1], &[1]).await;
        assert_eq!(keyed, 1);

        let zip = export_catalog(from.path(), downloads.path(), true)
            .await
            .unwrap();
        import_catalog(to.path(), &zip, false).await.unwrap();

        let completed = archive(to.path()).completed().unwrap();
        assert_eq!(completed.len(), 1);
        let mut texts = vec![fs::read_to_string(completed[0].dir().join("update.json")).unwrap()];
        for endpoint in [APP_LIST, ITEMS] {
            for record in completed[0].records(endpoint).unwrap() {
                let record = record.unwrap();
                texts.push(format!("{:?} {}", record.params, record.body));
            }
        }
        assert_eq!(texts.len(), 3);
        assert!(texts.iter().all(|text| !text.contains(KEY)));
        let zip = fs::read(&zip).unwrap();
        assert!(!zip.windows(KEY.len()).any(|bytes| bytes == KEY.as_bytes()));
    }

    #[tokio::test]
    async fn an_export_with_the_archive_needs_a_completed_update() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        assert!(matches!(
            export_catalog(data.path(), downloads.path(), true).await,
            Err(CatalogFileError::NoArchive)
        ));
        archive(data.path())
            .start_update(Timestamp(PRICES_AT), 0, Region::ITALY, false)
            .unwrap()
            .fail(Timestamp(PRICES_AT))
            .unwrap();
        assert!(matches!(
            export_catalog(data.path(), downloads.path(), true).await,
            Err(CatalogFileError::NoArchive)
        ));
        assert!(files(downloads.path()).is_empty());

        let zip = export_catalog(data.path(), downloads.path(), false)
            .await
            .unwrap();
        assert_eq!(zip_entries(&zip).len(), 1);
    }

    #[tokio::test]
    async fn an_open_chain_or_another_region_has_no_archive_to_export() {
        let open = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&open.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));
        archived_update(open.path(), 0, Region::ITALY, &[1, 2], &[2]).await;
        let american = exporter(Region::new("US").unwrap()).await;

        for data in [open.path(), american.path()] {
            assert!(matches!(
                export_catalog(data, downloads.path(), true).await,
                Err(CatalogFileError::NoArchive)
            ));
        }
    }

    fn zip_of(catalog: &Path, archive_dir: &Path, extra: Option<&str>, file: &Path) {
        let mut entries: Vec<(String, Vec<u8>)> =
            vec![(CATALOG.to_string(), fs::read(catalog).unwrap())];
        for folder in fs::read_dir(archive_dir).unwrap() {
            let folder = folder.unwrap();
            for update_file in fs::read_dir(folder.path()).unwrap() {
                let update_file = update_file.unwrap();
                entries.push((
                    format!(
                        "archive/{}/{}",
                        folder.file_name().to_string_lossy(),
                        update_file.file_name().to_string_lossy()
                    ),
                    fs::read(update_file.path()).unwrap(),
                ));
            }
        }
        if let Some(name) = extra {
            entries.push((name.to_string(), b"x".to_vec()));
        }
        let entries: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        zip_with(file, &entries);
    }

    #[tokio::test]
    async fn a_wrong_archive_leaves_only_the_imported_catalog() {
        let italian = exporter(Region::ITALY).await;
        let american = exporter(Region::new("US").unwrap()).await;
        let open = exporter(Region::ITALY).await;
        fs::remove_dir_all(open.path().join("archive").join("000001")).unwrap();
        let unfinished = exporter(Region::ITALY).await;
        archive(unfinished.path())
            .start_update(Timestamp(PRICES_AT), 0, Region::ITALY, false)
            .unwrap();
        let source = tempfile::tempdir().unwrap();
        let catalog = italian.path().join(CATALOG);
        let archive_of = |data: &tempfile::TempDir| data.path().join("archive");

        let cases = [
            ("dots", &italian, Some("archive/../x")),
            ("deep-dots", &italian, Some("archive/000001/../../x")),
            ("letters", &italian, Some("archive/abc/update.json")),
            ("backslash", &italian, Some("archive/000001\\update.json")),
            ("absolute", &italian, Some("archive//update.json")),
            ("loose", &italian, Some("archive/000001")),
            ("other-file", &italian, Some("archive/000001/notes.txt")),
            ("american", &american, None),
            ("open", &open, None),
            ("unfinished", &unfinished, None),
        ];
        for (name, exporter, extra) in cases {
            let zip = source.path().join(format!("{name}.zip"));
            zip_of(&catalog, &archive_of(exporter), extra, &zip);
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("data");

            let imported = import_catalog(&data, &zip, false).await.unwrap();

            assert_eq!(files(root.path()), ["data"], "{name}");
            assert_eq!(files(&data), ["catalog.sqlite", "update.lock"], "{name}");
            assert_only_the_catalog(&data, &imported);
        }
    }

    #[tokio::test]
    async fn a_failed_download_is_replaced_by_the_imported_archive() {
        let from = exporter(Region::ITALY).await;
        let to = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        archive(to.path())
            .start_update(Timestamp(PRICES_AT), 0, Region::ITALY, false)
            .unwrap()
            .fail(Timestamp(PRICES_AT))
            .unwrap();
        archive(to.path())
            .start_update(Timestamp(PRICES_AT), 0, Region::ITALY, false)
            .unwrap();

        let zip = export_catalog(from.path(), downloads.path(), true)
            .await
            .unwrap();
        let imported = import_catalog(to.path(), &zip, false).await.unwrap();

        assert_eq!(imported.archive, ArchiveImport::Imported { updates: 2 });
        assert_eq!(files(&to.path().join("archive")), ["000001", "000002"]);
        assert_eq!(archive(to.path()).completed().unwrap().len(), 2);
        assert!(!to.path().join(REPLACED_ARCHIVE).exists());
    }

    #[tokio::test]
    async fn a_completed_archive_is_replaced_only_when_asked() {
        let from = exporter(Region::ITALY).await;
        let to = exporter(Region::new("US").unwrap()).await;
        let downloads = tempfile::tempdir().unwrap();
        let zip = export_catalog(from.path(), downloads.path(), true)
            .await
            .unwrap();
        let regions = |data: &Path| -> Vec<Region> {
            archive(data)
                .completed()
                .unwrap()
                .iter()
                .map(ArchivedUpdate::region)
                .collect()
        };

        let kept = import_catalog(to.path(), &zip, false).await.unwrap();

        assert_only_the_catalog(to.path(), &kept);
        assert_eq!(regions(to.path()), [Region::new("US").unwrap(); 2]);

        let replaced = import_catalog(to.path(), &zip, true).await.unwrap();

        assert_eq!(replaced.archive, ArchiveImport::Imported { updates: 2 });
        assert_eq!(regions(to.path()), [Region::ITALY; 2]);
        assert!(!to.path().join(REPLACED_ARCHIVE).exists());
    }
}
