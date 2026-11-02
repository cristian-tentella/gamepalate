use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
};

use gamepalate_core::{
    catalog::{Catalog, Freshness, TagName},
    currency::Currency,
    game::{Discount, Game, GameTag, Price, Reviews},
    ids::{AppId, TagId},
    region::Region,
    timestamp::Timestamp,
};
use rusqlite::{Connection, OpenFlags, Row, params};

const CACHE_FORMAT: u32 = 2;

const SCHEMA: &str = "
    CREATE TABLE games (
        appid INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        final_price_cents INTEGER NOT NULL,
        original_price_cents INTEGER NOT NULL,
        discount_percent INTEGER NOT NULL,
        discount_end_ms INTEGER,
        review_count INTEGER NOT NULL,
        positive_count INTEGER NOT NULL,
        early_access INTEGER NOT NULL,
        price_change_number INTEGER NOT NULL
    );
    CREATE TABLE game_tags (
        appid INTEGER NOT NULL REFERENCES games(appid),
        tagid INTEGER NOT NULL,
        weight REAL NOT NULL,
        PRIMARY KEY (appid, tagid)
    ) WITHOUT ROWID;
    CREATE TABLE tags (tagid INTEGER PRIMARY KEY, name_it TEXT NOT NULL, name_en TEXT NOT NULL);
    CREATE TABLE tag_idf (tagid INTEGER PRIMARY KEY, idf REAL NOT NULL);
    CREATE TABLE freshness (
        prices_updated_at INTEGER NOT NULL,
        tags_updated_at INTEGER NOT NULL,
        region TEXT NOT NULL,
        currency_prefix TEXT NOT NULL,
        currency_suffix TEXT NOT NULL,
        decimal_separator TEXT,
        group_separator TEXT
    );
";

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the catalog")]
    Sqlite(#[from] rusqlite::Error),

    #[error("the catalog has an invalid region {0:?}")]
    Region(String),

    #[error("could not write the catalog")]
    Write(#[source] rusqlite::Error),

    #[error("could not replace the catalog at {}", .path.display())]
    Replace {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn load_catalog(path: &Path) -> Result<Catalog, CatalogError> {
    if !path.is_file() {
        return Err(CatalogError::NotFound(path.to_path_buf()));
    }

    let cache = path.with_extension("bin");
    if let Some(catalog) = read_cache(&cache, path) {
        return Ok(catalog);
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let catalog = read_catalog(&conn)?;
    write_cache(&cache, &catalog);

    Ok(catalog)
}

fn read_cache(cache: &Path, sqlite: &Path) -> Option<Catalog> {
    let cache_modified = cache.metadata().ok()?.modified().ok()?;
    let sqlite_modified = sqlite.metadata().ok()?.modified().ok()?;
    if cache_modified <= sqlite_modified {
        return None;
    }

    let bytes = std::fs::read(cache).ok()?;
    let (format, rest): (u32, _) = postcard::take_from_bytes(&bytes).ok()?;
    if format != CACHE_FORMAT {
        return None;
    }

    postcard::from_bytes(rest).ok()
}

fn write_cache(cache: &Path, catalog: &Catalog) {
    let Ok(bytes) = postcard::to_stdvec(&(CACHE_FORMAT, catalog)) else {
        return;
    };

    let temporary = temporary_path(cache);
    if std::fs::write(&temporary, bytes).is_err() || std::fs::rename(&temporary, cache).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

pub fn write_catalog(
    path: &Path,
    catalog: &Catalog,
    price_change_numbers: &HashMap<AppId, u64>,
) -> Result<(), CatalogError> {
    let temporary = temporary_path(path);
    let replace_error = |source| CatalogError::Replace {
        path: path.to_path_buf(),
        source,
    };

    match std::fs::remove_file(&temporary) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(replace_error(error)),
        _ => {}
    }

    let mut conn = Connection::open(&temporary).map_err(CatalogError::Write)?;
    insert_catalog(&mut conn, catalog, price_change_numbers).map_err(CatalogError::Write)?;
    conn.close()
        .map_err(|(_, error)| CatalogError::Write(error))?;

    std::fs::rename(&temporary, path).map_err(replace_error)
}

fn insert_catalog(
    conn: &mut Connection,
    catalog: &Catalog,
    price_change_numbers: &HashMap<AppId, u64>,
) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    let transaction = conn.transaction()?;

    {
        let mut insert_game = transaction.prepare_cached(
            "INSERT INTO games (appid, name, final_price_cents, original_price_cents,
                discount_percent, discount_end_ms, review_count, positive_count,
                early_access, price_change_number)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        let mut insert_game_tag = transaction
            .prepare_cached("INSERT INTO game_tags (appid, tagid, weight) VALUES (?, ?, ?)")?;

        for game in catalog.games() {
            let discount = game.price.discount;
            let price_change_number =
                i64::try_from(price_change_numbers.get(&game.id).copied().unwrap_or(0))
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            insert_game.execute(params![
                game.id.0,
                game.name,
                game.price.final_cents(),
                game.price.original_cents,
                discount.map_or(0, |discount| discount.percent),
                discount.and_then(|discount| discount.end).map(|end| end.0),
                game.reviews.total,
                game.reviews.positive,
                game.is_early_access,
                price_change_number,
            ])?;

            for tag in &game.tags {
                insert_game_tag.execute(params![game.id.0, tag.id.0, tag.weight])?;
            }
        }

        let mut insert_tag = transaction
            .prepare_cached("INSERT INTO tags (tagid, name_it, name_en) VALUES (?, ?, ?)")?;
        for (id, name) in &catalog.tags {
            insert_tag.execute(params![id.0, name.italian, name.english])?;
        }

        let mut insert_idf =
            transaction.prepare_cached("INSERT INTO tag_idf (tagid, idf) VALUES (?, ?)")?;
        for (id, idf) in &catalog.idf {
            insert_idf.execute(params![id.0, idf])?;
        }

        let currency = &catalog.currency;
        transaction.execute(
            "INSERT INTO freshness (prices_updated_at, tags_updated_at, region,
                currency_prefix, currency_suffix, decimal_separator, group_separator)
            VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                catalog.freshness.prices_updated_at.0,
                catalog.freshness.tags_updated_at.0,
                catalog.freshness.region.as_str(),
                currency.prefix,
                currency.suffix,
                currency.decimal_separator.map(String::from),
                currency.group_separator.map(String::from),
            ],
        )?;
    }

    transaction.commit()
}

fn read_catalog(conn: &Connection) -> Result<Catalog, CatalogError> {
    let mut games = read_games(conn)?;
    attach_game_tags(conn, &mut games)?;

    let tag_names = read_tag_names(conn)?;
    let idf = read_idf(conn)?;
    let (freshness, currency) = read_freshness(conn)?;

    Ok(Catalog::new(games, tag_names, idf, freshness, currency))
}

fn read_games(conn: &Connection) -> Result<Vec<Game>, CatalogError> {
    let mut stmt = conn.prepare(
        "SELECT appid, name, final_price_cents, original_price_cents,
            discount_percent, discount_end_ms, review_count,
            positive_count, early_access
        FROM games ORDER BY appid",
    )?;

    Ok(stmt
        .query_map([], game_from_row)?
        .collect::<Result<_, _>>()?)
}

// Columns by index, in the order of the SELECT in read_games
fn game_from_row(row: &Row) -> rusqlite::Result<Game> {
    Ok(Game {
        id: AppId(row.get(0)?),
        name: row.get(1)?,
        is_early_access: row.get(8)?,
        price: Price {
            original_cents: row.get(3)?,
            discount: discount_from_columns(row.get(4)?, row.get(2)?, row.get(5)?),
        },
        reviews: Reviews {
            total: row.get(6)?,
            positive: row.get(7)?,
        },
        tags: Vec::new(),
    })
}

fn discount_from_columns(percent: u8, final_cents: u32, end_ms: Option<i64>) -> Option<Discount> {
    if percent == 0 {
        return None;
    }

    Some(Discount {
        percent,
        final_cents,
        end: end_ms.map(Timestamp),
    })
}

fn attach_game_tags(conn: &Connection, games: &mut [Game]) -> Result<(), CatalogError> {
    let mut stmt =
        conn.prepare("SELECT appid, tagid, weight FROM game_tags ORDER BY appid, tagid")?;
    let mut rows = stmt.query([])?;
    let mut games = games.iter_mut().peekable();

    while let Some(row) = rows.next()? {
        let app_id = AppId(row.get(0)?);
        while games.next_if(|game| game.id < app_id).is_some() {}

        if let Some(game) = games.peek_mut().filter(|game| game.id == app_id) {
            game.tags.push(GameTag {
                id: TagId(row.get(1)?),
                weight: row.get(2)?,
            });
        }
    }

    Ok(())
}

fn read_tag_names(conn: &Connection) -> Result<BTreeMap<TagId, TagName>, CatalogError> {
    let mut stmt = conn.prepare("SELECT tagid, name_it, name_en FROM tags")?;

    Ok(stmt
        .query_map([], |row| {
            Ok((
                TagId(row.get(0)?),
                TagName {
                    english: row.get(2)?,
                    italian: row.get(1)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?)
}

fn read_idf(conn: &Connection) -> Result<BTreeMap<TagId, f64>, CatalogError> {
    let mut stmt = conn.prepare("SELECT tagid, idf FROM tag_idf")?;

    Ok(stmt
        .query_map([], |row| Ok((TagId(row.get(0)?), row.get(1)?)))?
        .collect::<Result<BTreeMap<_, _>, _>>()?)
}

fn read_freshness(conn: &Connection) -> Result<(Freshness, Currency), CatalogError> {
    if !has_region(conn)? {
        return Ok(conn.query_row(
            "SELECT prices_updated_at, tags_updated_at FROM freshness",
            [],
            |row| {
                Ok((
                    Freshness {
                        prices_updated_at: Timestamp(row.get(0)?),
                        tags_updated_at: Timestamp(row.get(1)?),
                        region: Region::ITALY,
                    },
                    Currency::italian(),
                ))
            },
        )?);
    }

    let (prices_updated_at, tags_updated_at, region, currency) = conn.query_row(
        "SELECT prices_updated_at, tags_updated_at, region,
            currency_prefix, currency_suffix, decimal_separator, group_separator
        FROM freshness",
        [],
        |row| {
            Ok((
                Timestamp(row.get(0)?),
                Timestamp(row.get(1)?),
                row.get::<_, String>(2)?,
                Currency {
                    prefix: row.get(3)?,
                    suffix: row.get(4)?,
                    decimal_separator: separator(row.get(5)?),
                    group_separator: separator(row.get(6)?),
                },
            ))
        },
    )?;
    let region = Region::new(&region).ok_or(CatalogError::Region(region))?;

    Ok((
        Freshness {
            prices_updated_at,
            tags_updated_at,
            region,
        },
        currency,
    ))
}

fn has_region(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare("SELECT 1 FROM pragma_table_info('freshness') WHERE name = 'region'")?
        .exists([])
}

fn separator(text: Option<String>) -> Option<char> {
    text.and_then(|text| text.chars().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATED_AT: i64 = 1_700_000_000_000;

    // Foreign keys are off so the tags of game 99, which is not in games, can be inserted.
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO games VALUES
                (30, 'Gamma', 800, 1000, 20, NULL, 0, 0, 0, 1),
                (10, 'Alpha', 1500, 1500, 0, NULL, 100, 90, 0, 3),
                (20, 'Beta', 1000, 2000, 50, 1700000000000, 300, 250, 1, 7);
            INSERT INTO game_tags VALUES
                (10, 5, 0.25),
                (10, 2, 0.75),
                (20, 2, 1.0),
                (99, 2, 1.0);
            INSERT INTO tags VALUES (2, 'Azione', 'Action'), (5, 'Indipendente', 'Indie');
            INSERT INTO tag_idf VALUES (2, 0.5), (5, 1.25);
            INSERT INTO freshness VALUES (1700000000000, 1690000000000, 'IT', '', '€', ',', NULL);",
        )
        .unwrap();
        conn
    }

    fn test_catalog() -> Catalog {
        read_catalog(&test_db()).unwrap()
    }

    #[test]
    fn discount_from_columns_without_percent_is_none() {
        assert_eq!(discount_from_columns(0, 1500, None), None);
    }

    #[test]
    fn discount_from_columns_keeps_the_end() {
        let discount = discount_from_columns(50, 1000, Some(UPDATED_AT));
        assert_eq!(
            discount,
            Some(Discount {
                percent: 50,
                final_cents: 1000,
                end: Some(Timestamp(UPDATED_AT)),
            })
        );
    }

    #[test]
    fn discount_from_columns_without_end_never_expires() {
        let discount = discount_from_columns(20, 800, None).unwrap();
        assert_eq!(discount.end, None);
    }

    #[test]
    fn read_catalog_returns_games_in_appid_order() {
        let ids: Vec<AppId> = test_catalog().games().iter().map(|game| game.id).collect();
        assert_eq!(ids, [AppId(10), AppId(20), AppId(30)]);
    }

    #[test]
    fn read_catalog_reads_every_column_of_a_game() {
        let catalog = test_catalog();
        let expected = Game {
            id: AppId(20),
            name: "Beta".to_string(),
            price: Price {
                original_cents: 2000,
                discount: Some(Discount {
                    percent: 50,
                    final_cents: 1000,
                    end: Some(Timestamp(UPDATED_AT)),
                }),
            },
            reviews: Reviews {
                total: 300,
                positive: 250,
            },
            is_early_access: true,
            tags: vec![GameTag {
                id: TagId(2),
                weight: 1.0,
            }],
        };
        assert_eq!(catalog.game(AppId(20)), Some(&expected));
    }

    #[test]
    fn read_catalog_attaches_tags_in_tagid_order() {
        let catalog = test_catalog();
        let tags = &catalog.game(AppId(10)).unwrap().tags;
        assert_eq!(
            tags,
            &[
                GameTag {
                    id: TagId(2),
                    weight: 0.75,
                },
                GameTag {
                    id: TagId(5),
                    weight: 0.25,
                },
            ]
        );
    }

    #[test]
    fn read_catalog_leaves_a_game_without_tags_empty() {
        let catalog = test_catalog();
        assert!(catalog.game(AppId(30)).unwrap().tags.is_empty());
    }

    #[test]
    fn read_catalog_ignores_tags_of_unknown_games() {
        let catalog = test_catalog();
        assert_eq!(catalog.game(AppId(99)), None);
        let tag_count: usize = catalog.games().iter().map(|game| game.tags.len()).sum();
        assert_eq!(tag_count, 3);
    }

    #[test]
    fn read_catalog_reads_tag_names_idf_and_freshness() {
        let catalog = test_catalog();

        let action = &catalog.tags[&TagId(2)];
        assert_eq!(action.italian, "Azione");
        assert_eq!(action.english, "Action");

        assert_eq!(catalog.idf[&TagId(5)], 1.25);

        assert_eq!(
            catalog.freshness,
            Freshness {
                prices_updated_at: Timestamp(UPDATED_AT),
                tags_updated_at: Timestamp(1_690_000_000_000),
                region: Region::ITALY,
            }
        );
        assert_eq!(catalog.currency, Currency::italian());
    }

    #[test]
    fn a_written_catalog_reads_back_the_same() {
        let catalog = test_catalog();
        let mut conn = Connection::open_in_memory().unwrap();

        insert_catalog(&mut conn, &catalog, &HashMap::new()).unwrap();
        let read = read_catalog(&conn).unwrap();

        assert_eq!(read.games(), catalog.games());
        assert_eq!(read.idf, catalog.idf);
        assert_eq!(read.freshness, catalog.freshness);
        assert_eq!(read.currency, catalog.currency);
        let names = |catalog: &Catalog| -> Vec<(TagId, String, String)> {
            catalog
                .tags
                .iter()
                .map(|(id, name)| (*id, name.english.clone(), name.italian.clone()))
                .collect()
        };
        assert_eq!(names(&read), names(&catalog));
    }

    #[test]
    fn a_written_catalog_keeps_the_price_change_number() {
        let catalog = test_catalog();
        let mut conn = Connection::open_in_memory().unwrap();

        insert_catalog(&mut conn, &catalog, &HashMap::from([(AppId(20), 42)])).unwrap();

        let numbers: Vec<(u32, i64)> = conn
            .prepare("SELECT appid, price_change_number FROM games ORDER BY appid")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(numbers, [(10, 0), (20, 42), (30, 0)]);
    }

    fn dollar_catalog() -> Catalog {
        let mut catalog = test_catalog();
        catalog.freshness.region = Region::new("US").unwrap();
        catalog.currency = Currency {
            prefix: "$".to_string(),
            suffix: String::new(),
            decimal_separator: Some('.'),
            group_separator: None,
        };
        catalog
    }

    #[test]
    fn a_written_catalog_keeps_its_region_and_currency() {
        let catalog = dollar_catalog();
        let mut conn = Connection::open_in_memory().unwrap();

        insert_catalog(&mut conn, &catalog, &HashMap::new()).unwrap();
        let read = read_catalog(&conn).unwrap();

        assert_eq!(read.freshness.region.as_str(), "US");
        assert_eq!(read.currency, catalog.currency);
    }

    #[test]
    fn a_catalog_without_region_and_currency_is_italian() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE games (
                appid INTEGER PRIMARY KEY, name TEXT NOT NULL, final_price_cents INTEGER NOT NULL,
                original_price_cents INTEGER NOT NULL, discount_percent INTEGER NOT NULL,
                discount_end_ms INTEGER, review_count INTEGER NOT NULL,
                positive_count INTEGER NOT NULL, early_access INTEGER NOT NULL,
                price_change_number INTEGER NOT NULL
            );
            CREATE TABLE game_tags (appid INTEGER NOT NULL, tagid INTEGER NOT NULL, weight REAL NOT NULL);
            CREATE TABLE tags (tagid INTEGER PRIMARY KEY, name_it TEXT NOT NULL, name_en TEXT NOT NULL);
            CREATE TABLE tag_idf (tagid INTEGER PRIMARY KEY, idf REAL NOT NULL);
            CREATE TABLE freshness (prices_updated_at INTEGER NOT NULL, tags_updated_at INTEGER NOT NULL);
            INSERT INTO games VALUES (10, 'Alpha', 1500, 1500, 0, NULL, 100, 90, 0, 3);
            INSERT INTO freshness VALUES (1700000000000, 1690000000000);",
        )
        .unwrap();

        let catalog = read_catalog(&conn).unwrap();

        assert_eq!(catalog.games().len(), 1);
        assert_eq!(catalog.freshness.region, Region::ITALY);
        assert_eq!(catalog.freshness.prices_updated_at, Timestamp(UPDATED_AT));
        assert_eq!(catalog.currency, Currency::italian());
    }

    #[test]
    fn an_invalid_region_is_an_error() {
        let conn = test_db();
        conn.execute_batch("UPDATE freshness SET region = 'usa'")
            .unwrap();

        assert!(matches!(
            read_catalog(&conn),
            Err(CatalogError::Region(region)) if region == "usa"
        ));
    }

    #[test]
    fn a_cache_of_another_format_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let sqlite = dir.path().join("catalog.sqlite");
        let cache = dir.path().join("catalog.bin");
        write_catalog(&sqlite, &test_catalog(), &HashMap::new()).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&sqlite)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        std::fs::write(
            &cache,
            postcard::to_stdvec(&(1_u32, &dollar_catalog())).unwrap(),
        )
        .unwrap();
        assert!(read_cache(&cache, &sqlite).is_none());

        write_cache(&cache, &dollar_catalog());
        assert_eq!(
            read_cache(&cache, &sqlite).unwrap().currency,
            dollar_catalog().currency
        );
    }

    #[test]
    fn load_catalog_reports_a_missing_file() {
        let result = load_catalog(Path::new("does-not-exist/catalogo.sqlite"));
        assert!(matches!(result, Err(CatalogError::NotFound(_))));
    }
}
