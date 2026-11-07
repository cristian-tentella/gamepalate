use std::path::{Path, PathBuf};

use gamepalate_core::{
    config::{Config, CorrectionsConfig, RelevanceConfig, ResultsConfig, SignalsConfig},
    ids::SteamId,
    region::Region,
};
use serde::Deserialize;
use toml_edit::DocumentMut;

use crate::atomic::write_atomically;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the config")]
    Io(#[from] std::io::Error),

    #[error("the config file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("the config file is not valid")]
    Edit(#[from] toml_edit::TomlError),

    #[error("could not save the config to {}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0} is missing from the config")]
    Missing(&'static str),

    #[error("config field {field} must be {expected}")]
    Invalid {
        field: &'static str,
        expected: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigValue {
    Number(f64),
    Integer(i64),
    Flag(bool),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct ConfigFile {
    steam_id: Option<String>,
    region: Option<String>,
    signals: SignalsSection,
    relevance: RelevanceSection,
    corrections: CorrectionsSection,
    results: ResultsSection,
    update: UpdateSection,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct SignalsSection {
    min_hours: Option<f64>,
    max_hours: Option<f64>,
    recent_hours_multiplier: Option<f64>,
    wishlist_hours: Option<f64>,
    followed_hours: Option<f64>,
    would_try_hours: Option<f64>,
    steam_client_hours: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct RelevanceSection {
    quality_weight: Option<f64>,
    min_reviews: Option<i64>,
    votes_weight: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct CorrectionsSection {
    boost: Option<f64>,
    dampen: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct ResultsSection {
    count: Option<i64>,
    early_access: Option<bool>,
    min_positive_percent: Option<i64>,
    bet_every: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct UpdateSection {
    prices_max_age_hours: Option<i64>,
    tags_max_age_days: Option<f64>,
    kept_updates: Option<i64>,
    renewal_requests: Option<i64>,
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    if !path.is_file() {
        return Err(ConfigError::NotFound(path.to_path_buf()));
    }

    let text = std::fs::read_to_string(path)?;

    parse_config(&text)
}

pub fn save_steam_id(path: &Path, steam_id: SteamId) -> Result<(), ConfigError> {
    save_value(
        path,
        &["steam_id"],
        toml_edit::value(steam_id.0.to_string()),
    )
}

pub fn save_region(path: &Path, region: Region) -> Result<(), ConfigError> {
    save_value(path, &["region"], toml_edit::value(region.as_str()))
}

pub fn save_steam_client_hours(path: &Path, read: bool) -> Result<(), ConfigError> {
    save_value(
        path,
        &["signals", "steam_client_hours"],
        toml_edit::value(read),
    )
}

pub fn save_config_value(
    path: &Path,
    keys: &[&str],
    value: ConfigValue,
) -> Result<(), ConfigError> {
    let value = match value {
        ConfigValue::Number(number) => toml_edit::value(number),
        ConfigValue::Integer(integer) => toml_edit::value(integer),
        ConfigValue::Flag(flag) => toml_edit::value(flag),
    };
    let mut document = read_document(path)?;
    set_value(&mut document, keys, value);
    parse_config(&document.to_string())?;

    write_document(path, &document)
}

pub fn remove_config_value(path: &Path, keys: &[&str]) -> Result<(), ConfigError> {
    let mut document = read_document(path)?;
    let (last, tables) = keys.split_last().expect("at least one key");
    let mut table = Some(document.as_table_mut() as &mut dyn toml_edit::TableLike);
    for key in tables {
        table = table
            .and_then(|table| table.get_mut(key))
            .and_then(|item| item.as_table_like_mut());
    }
    if let Some(table) = table {
        table.remove(last);
    }
    parse_config(&document.to_string())?;

    write_document(path, &document)
}

fn save_value(path: &Path, keys: &[&str], value: toml_edit::Item) -> Result<(), ConfigError> {
    let mut document = read_document(path)?;
    set_value(&mut document, keys, value);

    write_document(path, &document)
}

fn read_document(path: &Path) -> Result<DocumentMut, ConfigError> {
    let text = if path.is_file() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };

    Ok(text.parse()?)
}

fn set_value(document: &mut DocumentMut, keys: &[&str], value: toml_edit::Item) {
    let (last, tables) = keys.split_last().expect("at least one key");
    let mut item = document.as_item_mut();
    for key in tables {
        item = &mut item[*key];
        if item.is_none() {
            *item = toml_edit::table();
        }
    }
    item[*last] = value;
}

fn write_document(path: &Path, document: &DocumentMut) -> Result<(), ConfigError> {
    let write_error = |source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(write_error)?;
    }
    write_atomically(path, &document.to_string()).map_err(write_error)
}

fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let file: ConfigFile = toml::from_str(text)?;
    let base = Config::default();

    let steam_id = parse_steam_id(file.steam_id.as_deref())?;
    let region = match file.region.as_deref() {
        None => base.region,
        Some(code) => Region::new(code).ok_or_else(|| ConfigError::Invalid {
            field: "region",
            expected: "two letters, such as \"IT\"".to_string(),
        })?,
    };

    let signals = SignalsConfig {
        min_hours: number(
            "signals.min_hours",
            file.signals.min_hours,
            base.signals.min_hours,
            Range::AtLeast(0.0),
        )?,
        max_hours: number(
            "signals.max_hours",
            file.signals.max_hours,
            base.signals.max_hours,
            Range::Above(0.0),
        )?,
        recent_hours_multiplier: number(
            "signals.recent_hours_multiplier",
            file.signals.recent_hours_multiplier,
            base.signals.recent_hours_multiplier,
            Range::AtLeast(1.0),
        )?,
        wishlist_hours: number(
            "signals.wishlist_hours",
            file.signals.wishlist_hours,
            base.signals.wishlist_hours,
            Range::AtLeast(0.0),
        )?,
        followed_hours: number(
            "signals.followed_hours",
            file.signals.followed_hours,
            base.signals.followed_hours,
            Range::AtLeast(0.0),
        )?,
        would_try_hours: number(
            "signals.would_try_hours",
            file.signals.would_try_hours,
            base.signals.would_try_hours,
            Range::AtLeast(0.0),
        )?,
        steam_client_hours: file.signals.steam_client_hours,
    };

    let relevance = RelevanceConfig {
        quality_weight: number(
            "relevance.quality_weight",
            file.relevance.quality_weight,
            base.relevance.quality_weight,
            Range::Between(0.0, 1.0),
        )?,
        min_reviews: integer(
            "relevance.min_reviews",
            file.relevance.min_reviews,
            base.relevance.min_reviews,
            0,
            None,
        )?,
        votes_weight: number(
            "relevance.votes_weight",
            file.relevance.votes_weight,
            base.relevance.votes_weight,
            Range::AtLeast(0.0),
        )?,
    };

    let corrections = CorrectionsConfig {
        boost: number(
            "corrections.boost",
            file.corrections.boost,
            base.corrections.boost,
            Range::Above(0.0),
        )?,
        dampen: number(
            "corrections.dampen",
            file.corrections.dampen,
            base.corrections.dampen,
            Range::Above(0.0),
        )?,
    };

    let results = ResultsConfig {
        count: integer(
            "results.count",
            file.results.count,
            base.results.count,
            1,
            None,
        )?,
        early_access: file
            .results
            .early_access
            .unwrap_or(base.results.early_access),
        min_positive_percent: integer(
            "results.min_positive_percent",
            file.results.min_positive_percent,
            base.results.min_positive_percent,
            0,
            Some(100),
        )?,
        bet_every: integer(
            "results.bet_every",
            file.results.bet_every,
            base.results.bet_every,
            0,
            None,
        )?,
    };

    let prices_max_age_hours = integer(
        "update.prices_max_age_hours",
        file.update.prices_max_age_hours,
        base.prices_max_age_hours,
        1,
        None,
    )?;
    let tags_max_age_days = number(
        "update.tags_max_age_days",
        file.update.tags_max_age_days,
        base.tags_max_age_days,
        Range::Above(0.0),
    )?;
    let kept_updates = integer(
        "update.kept_updates",
        file.update.kept_updates,
        base.kept_updates,
        1,
        None,
    )?;
    let renewal_requests = integer(
        "update.renewal_requests",
        file.update.renewal_requests,
        base.renewal_requests,
        1,
        None,
    )?;

    Ok(Config {
        steam_id: Some(steam_id),
        signals,
        relevance,
        results,
        corrections,
        prices_max_age_hours,
        tags_max_age_days,
        kept_updates,
        renewal_requests,
        region,
    })
}

fn parse_steam_id(steam_id: Option<&str>) -> Result<SteamId, ConfigError> {
    match steam_id {
        None => Err(ConfigError::Missing("steam_id")),
        Some(id) if id.len() == 17 && id.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(SteamId(id.parse().expect("17 digits fit in a u64")))
        }
        Some(_) => Err(ConfigError::Invalid {
            field: "steam_id",
            expected: "a string of 17 digits".to_string(),
        }),
    }
}

#[derive(Clone, Copy)]
enum Range {
    AtLeast(f64),
    Above(f64),
    Between(f64, f64),
}

impl Range {
    fn contains(self, value: f64) -> bool {
        value.is_finite()
            && match self {
                Range::AtLeast(min) => value >= min,
                Range::Above(min) => value > min,
                Range::Between(min, max) => (min..=max).contains(&value),
            }
    }

    fn describe(self) -> String {
        match self {
            Range::AtLeast(min) => format!("a number >= {min}"),
            Range::Above(min) => format!("a number > {min}"),
            Range::Between(min, max) => format!("a number from {min} to {max}"),
        }
    }
}

fn number(
    field: &'static str,
    value: Option<f64>,
    default: f64,
    range: Range,
) -> Result<f64, ConfigError> {
    match value {
        None => Ok(default),
        Some(value) if range.contains(value) => Ok(value),
        Some(_) => Err(ConfigError::Invalid {
            field,
            expected: range.describe(),
        }),
    }
}

fn integer<T: TryFrom<i64>>(
    field: &'static str,
    value: Option<i64>,
    default: T,
    min: i64,
    max: Option<i64>,
) -> Result<T, ConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };

    let in_range = value >= min && max.is_none_or(|max| value <= max);
    match T::try_from(value) {
        Ok(value) if in_range => Ok(value),
        _ => Err(ConfigError::Invalid {
            field,
            expected: match max {
                Some(max) => format!("an integer from {min} to {max}"),
                None => format!("an integer >= {min}"),
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_ID: SteamId = SteamId(76_561_198_111_111_111);

    #[test]
    fn saving_the_steam_id_keeps_comments_and_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my settings\nsteam_id = \"76561198000000000\"\nregion = \"US\"\n\n[signals]\n# hours\nmin_hours = 3.0\n",
        )
        .unwrap();

        save_steam_id(&path, NEW_ID).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# my settings\nsteam_id = \"76561198111111111\"\nregion = \"US\"\n\n[signals]\n# hours\nmin_hours = 3.0\n"
        );
        let config = load_config(&path).unwrap();
        assert_eq!(config.steam_id, Some(NEW_ID));
        assert_eq!(config.signals.min_hours, 3.0);
    }

    #[test]
    fn saving_the_steam_id_creates_the_config_and_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gamepalate").join("config.toml");

        save_steam_id(&path, NEW_ID).unwrap();

        assert_eq!(load_config(&path).unwrap().steam_id, Some(NEW_ID));
    }

    #[test]
    fn a_steam_id_saved_before_the_tables_stays_at_the_top() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[signals]\nmin_hours = 3.0\n").unwrap();

        save_steam_id(&path, NEW_ID).unwrap();

        assert_eq!(load_config(&path).unwrap().steam_id, Some(NEW_ID));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("steam_id = ")
        );
    }

    #[test]
    fn saving_the_region_keeps_comments_and_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my settings\nsteam_id = \"76561198000000000\"\nregion = \"IT\"\n\n[signals]\nmin_hours = 3.0\n",
        )
        .unwrap();

        save_region(&path, Region::new("US").unwrap()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# my settings\nsteam_id = \"76561198000000000\"\nregion = \"US\"\n\n[signals]\nmin_hours = 3.0\n"
        );
    }

    #[test]
    fn a_region_saved_for_the_first_time_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "steam_id = \"76561198000000000\"\n[signals]\nmin_hours = 3.0\n",
        )
        .unwrap();

        save_region(&path, Region::new("US").unwrap()).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.region, Region::new("US").unwrap());
        assert_eq!(config.signals.min_hours, 3.0);
    }

    #[test]
    fn saving_the_steam_client_answer_keeps_the_signals_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my settings
steam_id = \"76561198000000000\"

[signals]
# hours
min_hours = 3.0
",
        )
        .unwrap();

        save_steam_client_hours(&path, true).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# my settings
steam_id = \"76561198000000000\"

[signals]
# hours
min_hours = 3.0
steam_client_hours = true
"
        );
        save_steam_client_hours(&path, false).unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.signals.steam_client_hours, Some(false));
        assert_eq!(config.signals.min_hours, 3.0);
    }

    const WITH_COMMENTS: &str = "# my settings
steam_id = \"76561198000000000\"

[signals]
# hours
min_hours = 3.0
";

    fn written(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn saving_a_value_keeps_comments_and_other_keys() {
        let (_dir, path) = written(WITH_COMMENTS);

        save_config_value(
            &path,
            &["signals", "would_try_hours"],
            ConfigValue::Number(3.0),
        )
        .unwrap();
        save_config_value(&path, &["results", "count"], ConfigValue::Integer(30)).unwrap();
        save_config_value(
            &path,
            &["results", "early_access"],
            ConfigValue::Flag(false),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# my settings
steam_id = \"76561198000000000\"

[signals]
# hours
min_hours = 3.0
would_try_hours = 3.0

[results]
count = 30
early_access = false
"
        );
        let config = load_config(&path).unwrap();
        assert_eq!(config.signals.would_try_hours, 3.0);
        assert_eq!(config.results.count, 30);
        assert!(!config.results.early_access);
    }

    #[test]
    fn a_value_out_of_range_is_not_written() {
        let (_dir, path) = written(WITH_COMMENTS);

        let result = save_config_value(
            &path,
            &["relevance", "quality_weight"],
            ConfigValue::Number(2.0),
        );

        assert_eq!(
            invalid_field(result.map(|()| Config::default())),
            "relevance.quality_weight"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), WITH_COMMENTS);
    }

    #[test]
    fn a_broken_file_is_not_touched() {
        let (_dir, path) = written("steam_id = \"123\"\n");

        let result = save_config_value(&path, &["results", "count"], ConfigValue::Integer(30));

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "steam_id = \"123\"\n"
        );
    }

    #[test]
    fn removing_a_value_brings_back_the_default() {
        let (_dir, path) = written(WITH_COMMENTS);

        remove_config_value(&path, &["signals", "min_hours"]).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# my settings
steam_id = \"76561198000000000\"

[signals]
"
        );
        assert_eq!(load_config(&path).unwrap().signals.min_hours, 2.0);
    }

    #[test]
    fn removing_a_value_that_is_not_there_changes_nothing() {
        let (_dir, path) = written(WITH_COMMENTS);

        remove_config_value(&path, &["signals", "max_hours"]).unwrap();
        remove_config_value(&path, &["results", "count"]).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), WITH_COMMENTS);
    }

    #[test]
    fn the_steam_client_answer_creates_the_signals_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "steam_id = \"76561198000000000\"
",
        )
        .unwrap();

        save_steam_client_hours(&path, true).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "steam_id = \"76561198000000000\"

[signals]
steam_client_hours = true
"
        );
        assert_eq!(
            load_config(&path).unwrap().signals.steam_client_hours,
            Some(true)
        );
    }

    const STEAM_ID: &str = r#"steam_id = "76561198000000000""#;

    fn parse_with(rest: &str) -> Result<Config, ConfigError> {
        parse_config(&format!("{STEAM_ID}\n{rest}"))
    }

    fn invalid_field(result: Result<Config, ConfigError>) -> &'static str {
        match result {
            Err(ConfigError::Invalid { field, .. }) => field,
            other => panic!("expected an invalid field, got {other:?}"),
        }
    }

    #[test]
    fn parse_config_with_only_the_steam_id_gives_the_defaults() {
        assert_eq!(
            parse_with("").unwrap(),
            Config {
                steam_id: Some(SteamId(76_561_198_000_000_000)),
                ..Config::default()
            }
        );
    }

    #[test]
    fn parse_config_reads_every_section() {
        let config = parse_with(
            "region = \"IT\"
            [signals]
            min_hours = 0.5
            would_try_hours = 8
            [relevance]
            votes_weight = 0.2
            [results]
            count = 10
            early_access = false
            min_positive_percent = 80
            [update]
            prices_max_age_hours = 12",
        )
        .unwrap();

        assert_eq!(config.signals.min_hours, 0.5);
        assert_eq!(config.signals.would_try_hours, 8.0);
        assert_eq!(config.signals.max_hours, 200.0);
        assert_eq!(config.relevance.votes_weight, 0.2);
        assert_eq!(config.results.count, 10);
        assert!(!config.results.early_access);
        assert_eq!(config.results.min_positive_percent, 80);
        assert_eq!(config.prices_max_age_hours, 12);
        assert_eq!(config.tags_max_age_days, 7.0);
        assert_eq!(config.kept_updates, 3);
        assert_eq!(config.renewal_requests, 150);
    }

    #[test]
    fn parse_config_reads_the_renewal_requests() {
        let config = parse_with(
            "[update]
renewal_requests = 40",
        )
        .unwrap();
        assert_eq!(config.renewal_requests, 40);

        let result = parse_with(
            "[update]
renewal_requests = 0",
        );
        assert_eq!(invalid_field(result), "update.renewal_requests");
    }

    #[test]
    fn parse_config_accepts_an_integer_where_a_number_is_expected() {
        let config = parse_with("[relevance]\nvotes_weight = 0").unwrap();
        assert_eq!(config.relevance.votes_weight, 0.0);
    }

    #[test]
    fn parse_config_requires_the_steam_id() {
        let result = parse_config("region = \"IT\"");
        assert!(matches!(result, Err(ConfigError::Missing("steam_id"))));
    }

    #[test]
    fn parse_config_rejects_a_short_steam_id() {
        let result = parse_config(r#"steam_id = "123""#);
        assert_eq!(invalid_field(result), "steam_id");
    }

    #[test]
    fn parse_config_reads_the_region() {
        assert_eq!(
            parse_with(r#"region = "US""#).unwrap().region,
            Region::new("US").unwrap()
        );
        assert_eq!(parse_with("").unwrap().region, Region::ITALY);
    }

    #[test]
    fn parse_config_rejects_a_region_that_is_not_two_letters() {
        let result = parse_with(r#"region = "USA""#);

        match result {
            Err(ConfigError::Invalid { field, expected }) => {
                assert_eq!(field, "region");
                assert_eq!(expected, "two letters, such as \"IT\"");
            }
            other => panic!("expected an invalid region, got {other:?}"),
        }
    }

    #[test]
    fn parse_config_names_the_field_out_of_range() {
        let result = parse_with("[results]\nmin_positive_percent = 101");
        assert_eq!(invalid_field(result), "results.min_positive_percent");

        let result = parse_with("[signals]\nrecent_hours_multiplier = 0.5");
        assert_eq!(invalid_field(result), "signals.recent_hours_multiplier");
    }

    #[test]
    fn parse_config_checks_fields_the_core_does_not_use() {
        let result = parse_with("[corrections]\nboost = 0");
        assert_eq!(invalid_field(result), "corrections.boost");
    }

    #[test]
    fn parse_config_rejects_an_unknown_key() {
        let result = parse_with("[results]\nlimit = 10");
        assert!(matches!(result, Err(ConfigError::Toml(_))));
    }

    #[test]
    fn parse_config_rejects_a_number_where_an_integer_is_expected() {
        let result = parse_with("[results]\ncount = 10.5");
        assert!(matches!(result, Err(ConfigError::Toml(_))));
    }

    #[test]
    fn load_config_reports_a_missing_file() {
        let result = load_config(Path::new("does-not-exist/config.toml"));
        assert!(matches!(result, Err(ConfigError::NotFound(_))));
    }
}
