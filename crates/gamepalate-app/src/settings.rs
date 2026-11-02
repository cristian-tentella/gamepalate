use std::path::{Path, PathBuf};

use gamepalate_core::{ids::SteamId, region::Region, timestamp::Timestamp};
use gamepalate_store::config::{ConfigError, save_region};

use crate::account::{CredentialManager, KEY_ENV_VAR, KeyStore};

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
}

pub fn change_region(data_dir: &Path, region: Region) -> Result<(), ConfigError> {
    save_region(&data_dir.join("config.toml"), region)
}

pub(crate) fn key_source() -> KeySource {
    key_source_with(std::env::var(KEY_ENV_VAR).ok(), &CredentialManager)
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
