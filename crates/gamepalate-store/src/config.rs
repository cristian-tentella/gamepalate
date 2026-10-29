use std::path::{Path, PathBuf};

use gamepalate_core::{
    config::{Config, CorrectionsConfig, RelevanceConfig, ResultsConfig, SignalsConfig},
    ids::SteamId,
};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config not found at {}", .0.display())]
    NotFound(PathBuf),

    #[error("could not read the config")]
    Io(#[from] std::io::Error),

    #[error("the config file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("{0} is missing from the config")]
    Missing(&'static str),

    #[error("config field {field} must be {expected}")]
    Invalid {
        field: &'static str,
        expected: String,
    },
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
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    if !path.is_file() {
        return Err(ConfigError::NotFound(path.to_path_buf()));
    }

    let text = std::fs::read_to_string(path)?;

    parse_config(&text)
}

fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let file: ConfigFile = toml::from_str(text)?;
    let base = Config::default();

    let steam_id = parse_steam_id(file.steam_id.as_deref())?;
    if file.region.as_deref().is_some_and(|region| region != "IT") {
        return Err(ConfigError::Invalid {
            field: "region",
            expected: "\"IT\"".to_string(),
        });
    }

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

    Ok(Config {
        steam_id: Some(steam_id),
        signals,
        relevance,
        results,
        corrections,
        prices_max_age_hours,
        tags_max_age_days,
        kept_updates,
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
    fn parse_config_rejects_another_region() {
        assert_eq!(invalid_field(parse_with(r#"region = "US""#)), "region");
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
