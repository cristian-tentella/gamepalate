use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use gamepalate_core::{
    catalog::{Catalog, Freshness, TagName},
    game::{Discount, Game, GameTag, Price, Reviews},
    ids::{AppId, TagId},
    timestamp::Timestamp,
};
use rusqlite::{Connection, OpenFlags, Row};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the catalog")]
    Sqlite(#[from] rusqlite::Error),
}

pub fn load_catalog(path: &Path) -> Result<Catalog, CatalogError> {
    if !path.is_file() {
        return Err(CatalogError::NotFound(path.to_path_buf()));
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    read_catalog(&conn)
}

fn read_catalog(conn: &Connection) -> Result<Catalog, CatalogError> {
    let mut games = read_games(conn)?;
    attach_game_tags(conn, &mut games)?;

    let tag_names = read_tag_names(conn)?;
    let idf = read_idf(conn)?;
    let freshness = read_freshness(conn)?;

    Ok(Catalog::new(games, tag_names, idf, freshness))
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

fn read_freshness(conn: &Connection) -> Result<Freshness, CatalogError> {
    Ok(conn.query_row(
        "SELECT prices_updated_at, tags_updated_at FROM freshness",
        [],
        |row| {
            Ok(Freshness {
                prices_updated_at: Timestamp(row.get(0)?),
                tags_updated_at: Timestamp(row.get(1)?),
            })
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATED_AT: i64 = 1_700_000_000_000;

    // Foreign keys are off so the tags of game 99, which is not in games, can be inserted.
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
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
            CREATE TABLE freshness (prices_updated_at INTEGER NOT NULL, tags_updated_at INTEGER NOT NULL);

            INSERT INTO games VALUES
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
            INSERT INTO freshness VALUES (1700000000000, 1690000000000);",
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
            }
        );
    }

    #[test]
    fn load_catalog_reports_a_missing_file() {
        let result = load_catalog(Path::new("does-not-exist/catalogo.sqlite"));
        assert!(matches!(result, Err(CatalogError::NotFound(_))));
    }
}
