use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use gamepalate_core::{region::Region, timestamp::Timestamp};
use gamepalate_store::catalog::{CatalogError, read_catalog_file, read_catalog_freshness};
use jiff::tz::TimeZone;
use zip::{CompressionMethod, ZipArchive, ZipWriter, result::ZipError, write::SimpleFileOptions};

use crate::lock::{LockError, UpdateLock};

const CATALOG: &str = "catalog.sqlite";
const CACHE: &str = "catalog.bin";
const IMPORT: &str = "import.sqlite";
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";
const MAX_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImportedCatalog {
    pub games: usize,
    pub prices_at: Timestamp,
    pub region: Region,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogFileError {
    #[error("This file has no catalog in it")]
    NoCatalog,

    #[error("This isn't a gamepalate catalog, or it comes from an older version")]
    NotACatalog(#[source] Option<CatalogError>),

    #[error("The catalog in this file is larger than 1 GB")]
    TooLarge,

    #[error("An update is running. Try again when it ends")]
    Locked,

    #[error("could not lock the data folder")]
    Lock(#[source] io::Error),

    #[error(transparent)]
    Catalog(#[from] CatalogError),

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

pub fn export_catalog(data_dir: &Path, into_dir: &Path) -> Result<PathBuf, CatalogFileError> {
    let _lock = lock(data_dir)?;
    let catalog = data_dir.join(CATALOG);
    let freshness = read_catalog_freshness(&catalog)?;
    let file = free_path(into_dir, &file_name(freshness.prices_updated_at));
    write_zip(&catalog, &file)?;
    Ok(file)
}

pub fn export_catalog_to(data_dir: &Path, file: &Path) -> Result<(), CatalogFileError> {
    let _lock = lock(data_dir)?;
    let catalog = data_dir.join(CATALOG);
    read_catalog_freshness(&catalog)?;
    write_zip(&catalog, file)
}

pub fn import_catalog(data_dir: &Path, file: &Path) -> Result<ImportedCatalog, CatalogFileError> {
    let _lock = lock(data_dir)?;
    let imported = data_dir.join(IMPORT);
    let result = place_catalog(file, &imported, data_dir);
    if result.is_err() {
        let _ = fs::remove_file(&imported);
    }
    result
}

fn lock(data_dir: &Path) -> Result<UpdateLock, CatalogFileError> {
    UpdateLock::take(data_dir).map_err(|error| match error {
        LockError::Taken => CatalogFileError::Locked,
        LockError::Io(source) => CatalogFileError::Lock(source),
    })
}

fn file_name(prices_at: Timestamp) -> String {
    let date = jiff::Timestamp::from_millisecond(prices_at.0)
        .map(|time| time.to_zoned(TimeZone::system()).date().to_string())
        .unwrap_or_default();
    format!("gamepalate-catalog-{date}")
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

fn write_zip(catalog: &Path, file: &Path) -> Result<(), CatalogFileError> {
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let result = zip_into(catalog, &temporary).and_then(|()| {
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

fn zip_into(catalog: &Path, temporary: &Path) -> Result<(), CatalogFileError> {
    let write_error = |source| CatalogFileError::Write {
        path: temporary.to_path_buf(),
        source,
    };
    let mut source = File::open(catalog).map_err(|source| CatalogFileError::Read {
        path: catalog.to_path_buf(),
        source,
    })?;

    let mut zip = ZipWriter::new(File::create(temporary).map_err(write_error)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(CATALOG, options)
        .map_err(|error| write_error(error.into()))?;
    io::copy(&mut source, &mut zip).map_err(write_error)?;
    zip.finish()
        .map_err(|error| write_error(error.into()))?
        .sync_all()
        .map_err(write_error)
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
    })
}

fn copy_catalog(file: &Path, imported: &Path) -> Result<(), CatalogFileError> {
    let read_error = |source| CatalogFileError::Read {
        path: file.to_path_buf(),
        source,
    };
    let mut source = File::open(file).map_err(read_error)?;
    let mut magic = [0; 4];
    let is_zip = match source.read_exact(&mut magic) {
        Ok(()) => &magic == ZIP_MAGIC,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => false,
        Err(error) => return Err(read_error(error)),
    };

    if is_zip {
        let mut archive = ZipArchive::new(source).map_err(CatalogFileError::Zip)?;
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
        let source = File::open(file).map_err(read_error)?;
        write_limited(source, imported)
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use gamepalate_core::{
        catalog::{Catalog, Freshness},
        currency::Currency,
        game::{Game, Price, Reviews},
        ids::AppId,
    };
    use gamepalate_steam::archive::Archive;
    use gamepalate_store::catalog::{load_catalog, write_catalog};

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

    #[test]
    fn an_exported_catalog_imports_back_with_the_same_games() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(
            &from.path().join(CATALOG),
            &catalog(&["Alpha", "Beta"], PRICES_AT),
        );

        let zip = export_catalog(from.path(), downloads.path()).unwrap();
        let imported = import_catalog(to.path(), &zip).unwrap();

        assert_eq!(
            imported,
            ImportedCatalog {
                games: 2,
                prices_at: Timestamp(PRICES_AT),
                region: Region::ITALY,
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

    #[test]
    fn the_file_name_has_the_date_of_the_prices_and_a_copy_number() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        let first = export_catalog(data.path(), downloads.path()).unwrap();
        let second = export_catalog(data.path(), downloads.path()).unwrap();
        let third = export_catalog(data.path(), downloads.path()).unwrap();

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

    #[test]
    fn the_zip_holds_only_the_catalog() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        let zip = export_catalog(data.path(), downloads.path()).unwrap();
        let mut archive = ZipArchive::new(File::open(zip).unwrap()).unwrap();

        assert_eq!(archive.len(), 1);
        let entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name().unwrap(), CATALOG);
        assert_eq!(entry.compression(), CompressionMethod::Deflated);
    }

    #[test]
    fn export_to_a_named_file() {
        let data = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let file = target.path().join("friend.zip");
        write(&data.path().join(CATALOG), &catalog(&["Alpha"], PRICES_AT));

        export_catalog_to(data.path(), &file).unwrap();

        assert_eq!(files(target.path()), ["friend.zip"]);
    }

    #[test]
    fn a_loose_catalog_imports_into_a_missing_data_folder() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("gamepalate");
        let loose = source.path().join("catalog.sqlite");
        write(&loose, &catalog(&["Alpha"], PRICES_AT));

        let imported = import_catalog(&data, &loose).unwrap();

        assert_eq!(imported.games, 1);
        assert_eq!(files(&data), ["catalog.sqlite", "update.lock"]);
    }

    #[test]
    fn a_catalog_in_a_zip_subfolder_imports() {
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

        assert_eq!(import_catalog(data.path(), &zip).unwrap().games, 1);
    }

    #[test]
    fn a_wrong_file_leaves_the_folder_as_it_was() {
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
            import_catalog(data.path(), &no_catalog),
            Err(CatalogFileError::NoCatalog)
        ));
        for wrong in [&text, &empty_sqlite, &empty_file] {
            assert!(matches!(
                import_catalog(data.path(), wrong),
                Err(CatalogFileError::NotACatalog(_))
            ));
        }
        assert!(matches!(
            import_catalog(data.path(), &source.path().join("missing.zip")),
            Err(CatalogFileError::Read { .. })
        ));

        assert_eq!(
            files(data.path()),
            ["catalog.bin", "catalog.sqlite", "update.lock"]
        );
        assert_eq!(fs::read(data.path().join(CATALOG)).unwrap(), before);
    }

    #[test]
    fn an_import_removes_the_cache_even_when_the_file_is_older() {
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
        import_catalog(data.path(), &loose).unwrap();

        assert!(!data.path().join(CACHE).exists());
        assert_eq!(names(&load_catalog(&sqlite).unwrap()), ["New"]);
    }

    #[test]
    fn export_and_import_wait_for_a_running_update() {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let sqlite = data.path().join(CATALOG);
        write(&sqlite, &catalog(&["Alpha"], PRICES_AT));
        let held = UpdateLock::take(data.path()).unwrap();

        assert!(matches!(
            export_catalog(data.path(), downloads.path()),
            Err(CatalogFileError::Locked)
        ));
        assert!(matches!(
            import_catalog(data.path(), &sqlite),
            Err(CatalogFileError::Locked)
        ));
        assert!(files(downloads.path()).is_empty());

        drop(held);
        export_catalog(data.path(), downloads.path()).unwrap();
    }

    #[test]
    fn an_imported_catalog_has_no_completed_update() {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let loose = source.path().join("catalog.sqlite");
        write(&loose, &catalog(&["Alpha"], PRICES_AT));

        import_catalog(data.path(), &loose).unwrap();
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
}
