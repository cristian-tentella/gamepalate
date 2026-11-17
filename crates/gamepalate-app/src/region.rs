use std::path::Path;

use gamepalate_core::region::Region;
pub use gamepalate_core::region::countries;
use gamepalate_steam::{
    signals::get_player_summary,
    transport::{Get, Transport},
};
use gamepalate_store::config::load_config;

use crate::{account::steam_api_key, log::failed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSource {
    Profile,
    System,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartingRegion {
    pub region: Region,
    pub source: RegionSource,
}

pub fn starting_region(profile: Option<Region>, system: Option<Region>) -> StartingRegion {
    let known = |region: Option<Region>| region.filter(|region| region.country_name().is_some());
    match (known(profile), known(system)) {
        (Some(region), _) => StartingRegion {
            region,
            source: RegionSource::Profile,
        },
        (None, Some(region)) => StartingRegion {
            region,
            source: RegionSource::System,
        },
        (None, None) => StartingRegion {
            region: Region::ITALY,
            source: RegionSource::Default,
        },
    }
}

pub fn system_country() -> Option<Region> {
    sys_locale::get_locale().and_then(|locale| country_of_locale(&locale))
}

fn country_of_locale(locale: &str) -> Option<Region> {
    let name = locale.split(['.', '@']).next()?;
    let country = name.split(['_', '-']).skip(1).last()?;
    Region::new(&country.to_ascii_uppercase())
}

pub async fn profile_country(data_dir: &Path) -> Option<Region> {
    let steam_id = load_config(&data_dir.join("config.toml")).ok()?.steam_id?;
    let key = steam_api_key()?;
    let steam = Transport::new(Some(key))
        .inspect_err(failed("could not reach Steam for the region"))
        .ok()?;
    country_in_profile(&steam, steam_id).await
}

async fn country_in_profile<S: Get>(
    steam: &S,
    steam_id: gamepalate_core::ids::SteamId,
) -> Option<Region> {
    get_player_summary(steam, steam_id)
        .await
        .inspect_err(failed("could not read the Steam profile country"))
        .ok()
        .flatten()
        .and_then(|profile| profile.country)
}

#[cfg(test)]
mod tests {
    use gamepalate_core::ids::SteamId;
    use gamepalate_steam::transport::SteamError;
    use serde_json::{Value, json};

    use super::*;

    fn region(code: &str) -> Option<Region> {
        Region::new(code)
    }

    #[test]
    fn the_profile_comes_first_then_the_system_then_italy() {
        let from = |profile, system| starting_region(profile, system);

        assert_eq!(
            from(region("DE"), region("US")),
            StartingRegion {
                region: Region::new("DE").unwrap(),
                source: RegionSource::Profile
            }
        );
        assert_eq!(
            from(region("DE"), None),
            StartingRegion {
                region: Region::new("DE").unwrap(),
                source: RegionSource::Profile
            }
        );
        assert_eq!(
            from(None, region("US")),
            StartingRegion {
                region: Region::new("US").unwrap(),
                source: RegionSource::System
            }
        );
        assert_eq!(
            from(None, None),
            StartingRegion {
                region: Region::ITALY,
                source: RegionSource::Default
            }
        );
    }

    #[test]
    fn a_country_outside_the_table_does_not_count() {
        assert_eq!(
            starting_region(region("ZZ"), region("US")),
            StartingRegion {
                region: Region::new("US").unwrap(),
                source: RegionSource::System
            }
        );
        assert_eq!(
            starting_region(region("ZZ"), region("QQ")),
            StartingRegion {
                region: Region::ITALY,
                source: RegionSource::Default
            }
        );
    }

    #[test]
    fn a_locale_gives_its_country_when_it_has_one() {
        assert_eq!(country_of_locale("it_IT.UTF-8"), region("IT"));
        assert_eq!(country_of_locale("en_US"), region("US"));
        assert_eq!(country_of_locale("en-GB"), region("GB"));
        assert_eq!(country_of_locale("pt_BR@euro"), region("BR"));
        assert_eq!(country_of_locale("zh-Hans-CN"), region("CN"));
        for locale in ["C", "POSIX", "en", "C.UTF-8", ""] {
            assert_eq!(country_of_locale(locale), None, "{locale:?}");
        }
    }

    struct FakeSteam(Result<Value, ()>);

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            self.0.clone().map_err(|()| SteamError::SteamDown {
                endpoint: endpoint.to_string(),
                reason: "timed out".to_string(),
                source: None,
            })
        }
    }

    const ME: SteamId = SteamId(76_561_198_000_000_000);

    #[tokio::test]
    async fn the_profile_country_comes_from_the_summary() {
        let steam = FakeSteam(Ok(json!({ "response": { "players": [
            { "steamid": "76561198000000000", "loccountrycode": "FR" }
        ] } })));

        assert_eq!(country_in_profile(&steam, ME).await, region("FR"));
    }

    #[tokio::test]
    async fn an_error_or_a_profile_without_a_country_gives_none() {
        let without = FakeSteam(Ok(json!({ "response": { "players": [
            { "steamid": "76561198000000000" }
        ] } })));

        assert_eq!(country_in_profile(&without, ME).await, None);
        assert_eq!(country_in_profile(&FakeSteam(Err(())), ME).await, None);
    }
}
