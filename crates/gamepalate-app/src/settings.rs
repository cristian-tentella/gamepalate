use std::path::{Path, PathBuf};

use gamepalate_core::{config::Config, ids::SteamId, region::Region, timestamp::Timestamp};
use gamepalate_store::config::{
    ConfigError, ConfigValue, remove_config_value, save_config_value, save_region,
};

use crate::{
    account::{KEY_ENV_VAR, KeyStore, SystemKeyring},
    log::failed,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Environment,
    Saved,
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub steam_id: Option<SteamId>,
    pub key: KeySource,
    pub region: Region,
    pub data_dir: PathBuf,
    pub image_cache_max_bytes: u64,
    pub steam_client_hours: Option<bool>,
    pub steam_client_file: Option<PathBuf>,
    pub catalog_games: usize,
    pub tags_updated_at: Timestamp,
    pub config: Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    WishlistHours,
    FollowedHours,
    WouldTryHours,
    ElsewhereHours,
    MinHours,
    MaxHours,
    RecentHoursMultiplier,
    Count,
    EarlyAccess,
    MinReviews,
    MinPositivePercent,
    BetEvery,
    QualityWeight,
    VotesWeight,
    Boost,
    Dampen,
}

impl ConfigKey {
    pub fn keys(self) -> [&'static str; 2] {
        match self {
            ConfigKey::WishlistHours => ["signals", "wishlist_hours"],
            ConfigKey::FollowedHours => ["signals", "followed_hours"],
            ConfigKey::WouldTryHours => ["signals", "would_try_hours"],
            ConfigKey::ElsewhereHours => ["signals", "elsewhere_hours"],
            ConfigKey::MinHours => ["signals", "min_hours"],
            ConfigKey::MaxHours => ["signals", "max_hours"],
            ConfigKey::RecentHoursMultiplier => ["signals", "recent_hours_multiplier"],
            ConfigKey::Count => ["results", "count"],
            ConfigKey::EarlyAccess => ["results", "early_access"],
            ConfigKey::MinReviews => ["relevance", "min_reviews"],
            ConfigKey::MinPositivePercent => ["results", "min_positive_percent"],
            ConfigKey::BetEvery => ["results", "bet_every"],
            ConfigKey::QualityWeight => ["relevance", "quality_weight"],
            ConfigKey::VotesWeight => ["relevance", "votes_weight"],
            ConfigKey::Boost => ["corrections", "boost"],
            ConfigKey::Dampen => ["corrections", "dampen"],
        }
    }

    pub fn value(self, config: &Config) -> ConfigValue {
        let integer = |value: usize| ConfigValue::Integer(value as i64);
        match self {
            ConfigKey::WishlistHours => ConfigValue::Number(config.signals.wishlist_hours),
            ConfigKey::FollowedHours => ConfigValue::Number(config.signals.followed_hours),
            ConfigKey::WouldTryHours => ConfigValue::Number(config.signals.would_try_hours),
            ConfigKey::ElsewhereHours => ConfigValue::Number(config.signals.elsewhere_hours),
            ConfigKey::MinHours => ConfigValue::Number(config.signals.min_hours),
            ConfigKey::MaxHours => ConfigValue::Number(config.signals.max_hours),
            ConfigKey::RecentHoursMultiplier => {
                ConfigValue::Number(config.signals.recent_hours_multiplier)
            }
            ConfigKey::Count => integer(config.results.count),
            ConfigKey::EarlyAccess => ConfigValue::Flag(config.results.early_access),
            ConfigKey::MinReviews => ConfigValue::Integer(config.relevance.min_reviews.into()),
            ConfigKey::MinPositivePercent => {
                ConfigValue::Integer(config.results.min_positive_percent.into())
            }
            ConfigKey::BetEvery => integer(config.results.bet_every),
            ConfigKey::QualityWeight => ConfigValue::Number(config.relevance.quality_weight),
            ConfigKey::VotesWeight => ConfigValue::Number(config.relevance.votes_weight),
            ConfigKey::Boost => ConfigValue::Number(config.corrections.boost),
            ConfigKey::Dampen => ConfigValue::Number(config.corrections.dampen),
        }
    }

    pub fn default_value(self) -> ConfigValue {
        self.value(&Config::default())
    }
}

pub fn change_config(
    data_dir: &Path,
    key: ConfigKey,
    value: ConfigValue,
) -> Result<(), ConfigError> {
    save_config_value(&data_dir.join("config.toml"), &key.keys(), value)
        .inspect_err(failed("could not save config.toml"))
}

pub fn reset_config(data_dir: &Path, key: ConfigKey) -> Result<(), ConfigError> {
    remove_config_value(&data_dir.join("config.toml"), &key.keys())
        .inspect_err(failed("could not save config.toml"))
}

pub fn change_region(data_dir: &Path, region: Region) -> Result<(), ConfigError> {
    save_region(&data_dir.join("config.toml"), region)
        .inspect_err(failed("could not save config.toml"))
}

pub(crate) fn key_source() -> KeySource {
    key_source_with(std::env::var(KEY_ENV_VAR).ok(), &SystemKeyring)
}

fn key_source_with(env: Option<String>, store: &dyn KeyStore) -> KeySource {
    if env.is_some_and(|key| !key.trim().is_empty()) {
        return KeySource::Environment;
    }
    match store.load() {
        Ok(Some(_)) => KeySource::Saved,
        Ok(None) | Err(_) => KeySource::Missing,
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_store::config::load_config;

    use super::*;
    use crate::account::KeyError;

    struct FakeKeys(Option<&'static str>);

    impl KeyStore for FakeKeys {
        fn load(&self) -> Result<Option<String>, KeyError> {
            Ok(self.0.map(str::to_string))
        }

        fn save(&self, _key: &str) -> Result<(), KeyError> {
            Ok(())
        }
    }

    struct BrokenKeys;

    impl KeyStore for BrokenKeys {
        fn load(&self) -> Result<Option<String>, KeyError> {
            Err(KeyError::Store("no store".to_string()))
        }

        fn save(&self, _key: &str) -> Result<(), KeyError> {
            Ok(())
        }
    }

    #[test]
    fn the_environment_wins_over_the_saved_key() {
        assert_eq!(
            key_source_with(Some("ENV".to_string()), &FakeKeys(Some("SAVED"))),
            KeySource::Environment
        );
        assert_eq!(
            key_source_with(Some(" ".to_string()), &FakeKeys(Some("SAVED"))),
            KeySource::Saved
        );
        assert_eq!(
            key_source_with(None, &FakeKeys(Some("SAVED"))),
            KeySource::Saved
        );
    }

    #[test]
    fn no_key_or_a_broken_store_is_a_missing_key() {
        assert_eq!(key_source_with(None, &FakeKeys(None)), KeySource::Missing);
        assert_eq!(key_source_with(None, &BrokenKeys), KeySource::Missing);
    }

    #[test]
    fn a_changed_value_is_read_back_and_a_reset_one_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "steam_id = \"76561198000000000\"\n").unwrap();

        change_config(
            dir.path(),
            ConfigKey::WouldTryHours,
            ConfigValue::Number(3.0),
        )
        .unwrap();
        change_config(dir.path(), ConfigKey::MinReviews, ConfigValue::Integer(500)).unwrap();
        change_config(dir.path(), ConfigKey::EarlyAccess, ConfigValue::Flag(false)).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(
            ConfigKey::WouldTryHours.value(&config),
            ConfigValue::Number(3.0)
        );
        assert_eq!(
            ConfigKey::MinReviews.value(&config),
            ConfigValue::Integer(500)
        );
        assert_eq!(
            ConfigKey::EarlyAccess.value(&config),
            ConfigValue::Flag(false)
        );

        reset_config(dir.path(), ConfigKey::WouldTryHours).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(
            ConfigKey::WouldTryHours.value(&config),
            ConfigKey::WouldTryHours.default_value()
        );
        assert_eq!(
            ConfigKey::MinReviews.value(&config),
            ConfigValue::Integer(500)
        );
    }

    #[test]
    fn the_hours_of_a_game_played_elsewhere_are_read_and_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "steam_id = \"76561198000000000\"
",
        )
        .unwrap();
        assert_eq!(
            ConfigKey::ElsewhereHours.default_value(),
            ConfigValue::Number(20.0)
        );

        change_config(
            dir.path(),
            ConfigKey::ElsewhereHours,
            ConfigValue::Number(35.0),
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.signals.elsewhere_hours, 35.0);
        assert_eq!(
            ConfigKey::ElsewhereHours.value(&config),
            ConfigValue::Number(35.0)
        );
        reset_config(dir.path(), ConfigKey::ElsewhereHours).unwrap();
        assert_eq!(load_config(&path).unwrap().signals.elsewhere_hours, 20.0);
    }

    #[test]
    fn every_key_writes_a_value_the_config_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "steam_id = \"76561198000000000\"\n").unwrap();
        let keys = [
            ConfigKey::WishlistHours,
            ConfigKey::FollowedHours,
            ConfigKey::WouldTryHours,
            ConfigKey::ElsewhereHours,
            ConfigKey::MinHours,
            ConfigKey::MaxHours,
            ConfigKey::RecentHoursMultiplier,
            ConfigKey::Count,
            ConfigKey::EarlyAccess,
            ConfigKey::MinReviews,
            ConfigKey::MinPositivePercent,
            ConfigKey::BetEvery,
            ConfigKey::QualityWeight,
            ConfigKey::VotesWeight,
            ConfigKey::Boost,
            ConfigKey::Dampen,
        ];

        for key in keys {
            change_config(dir.path(), key, key.default_value()).unwrap();
        }

        assert_eq!(
            load_config(&path).unwrap(),
            Config {
                steam_id: Some(SteamId(76_561_198_000_000_000)),
                ..Config::default()
            }
        );
    }

    #[test]
    fn changing_the_region_writes_only_the_region() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "steam_id = \"76561198000000000\"\n[signals]\nmin_hours = 3.0\n",
        )
        .unwrap();

        change_region(dir.path(), Region::new("US").unwrap()).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.region, Region::new("US").unwrap());
        assert_eq!(config.signals.min_hours, 3.0);
    }
}
