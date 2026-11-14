use std::{collections::HashSet, sync::Mutex};

use gamepalate_core::ids::AppId;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const REPORT_ENDPOINT: &str =
    "https://store.steampowered.com/saleaction/ajaxgetdeckappcompatibilityreport";
const PROTONDB_ENDPOINT: &str = "https://www.protondb.com/api/v1/reports/summaries";
const SUCCESS: u32 = 1;
const TOKEN_PREFIX: &str = "#SteamDeckVerified_TestResult_";

static UNKNOWN_TOKENS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    Passed,
    Warning,
    Failed,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckCheck {
    pub result: CheckResult,
    pub text: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtonTier {
    Platinum,
    Gold,
    Silver,
    Bronze,
    Borked,
    Native,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtonDb {
    pub tier: ProtonTier,
    pub reports: u32,
}

#[derive(Deserialize)]
struct ReportEnvelope {
    success: u32,
    #[serde(default)]
    results: Option<ReportResults>,
}

#[derive(Deserialize)]
struct ReportResults {
    #[serde(default)]
    resolved_items: Vec<ReportItem>,
}

#[derive(Deserialize)]
struct ReportItem {
    display_type: u32,
    loc_token: String,
}

#[derive(Deserialize)]
struct Summary {
    tier: String,
    #[serde(default)]
    total: u32,
}

pub async fn get_deck_report<S: Get>(
    transport: &S,
    id: AppId,
) -> Result<Vec<DeckCheck>, SteamError> {
    let appid = id.0.to_string();
    let value = transport
        .get(
            REPORT_ENDPOINT,
            &[("nAppID", &appid), ("l", "english")],
            false,
        )
        .await?;
    parse_report(value)
}

pub async fn get_protondb<S: Get>(
    transport: &S,
    id: AppId,
) -> Result<Option<ProtonDb>, SteamError> {
    let endpoint = format!("{PROTONDB_ENDPOINT}/{}.json", id.0);
    match transport.get(&endpoint, &[], false).await {
        Ok(value) => parse_summary(&endpoint, value).map(Some),
        Err(SteamError::Status { status, .. }) if status == StatusCode::NOT_FOUND => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_report(value: Value) -> Result<Vec<DeckCheck>, SteamError> {
    let envelope: ReportEnvelope = parse(REPORT_ENDPOINT, value)?;
    if envelope.success != SUCCESS {
        return Ok(Vec::new());
    }

    let items = envelope
        .results
        .map(|r| r.resolved_items)
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let key = item.loc_token.strip_prefix(TOKEN_PREFIX)?;
            let Some(text) = deck_text(key) else {
                warn_once(key);
                return None;
            };
            Some(DeckCheck {
                result: match item.display_type {
                    4 => CheckResult::Passed,
                    3 => CheckResult::Warning,
                    2 => CheckResult::Failed,
                    _ => CheckResult::Note,
                },
                text,
            })
        })
        .collect())
}

fn parse_summary(endpoint: &str, value: Value) -> Result<ProtonDb, SteamError> {
    let summary: Summary = parse(endpoint, value)?;
    Ok(ProtonDb {
        tier: match summary.tier.as_str() {
            "platinum" => ProtonTier::Platinum,
            "gold" => ProtonTier::Gold,
            "silver" => ProtonTier::Silver,
            "bronze" => ProtonTier::Bronze,
            "borked" => ProtonTier::Borked,
            "native" => ProtonTier::Native,
            _ => ProtonTier::Pending,
        },
        reports: summary.total,
    })
}

fn warn_once(key: &str) {
    let mut seen = UNKNOWN_TOKENS.lock().unwrap_or_else(|e| e.into_inner());
    if seen
        .get_or_insert_with(HashSet::new)
        .insert(key.to_string())
    {
        log::warn!("no text for the Steam Deck report key {key}");
    }
}

fn deck_text(key: &str) -> Option<&'static str> {
    Some(match key {
        "AuxFunctionalityNotAccessible_MapEditor" => "The map editor doesn't work on Steam Deck",
        "ControllerGlyphsDoNotMatchDeckDevice" => "Shows buttons from another controller",
        "ControllerGlyphsMatchDeckDevice" => "Shows the Steam Deck's buttons",
        "CrossPlatformCloudSavesNotSupported" => "Cloud saves don't sync with other platforms",
        "DefaultConfigurationIsNotPerformant" => "Needs lower settings to run well",
        "DefaultConfigurationIsPerformant" => "Runs well with default settings",
        "DefaultControllerConfigFullyFunctional" => "Everything works with the controller",
        "DefaultControllerConfigNotFullyFunctional" => {
            "Some actions need the touchscreen or a keyboard"
        }
        "DeviceCompatibilityWarningsShown" => "Shows compatibility warnings when it starts",
        "DisplayOutputNotCorrectlyScaled" => "Doesn't fit the screen correctly",
        "ExternalControllersNotSupportedPrimaryPlayer" => "Doesn't work with external controllers",
        "FirstTimeSetupRequiresActiveInternetConnection" => {
            "Needs an internet connection the first time"
        }
        "GameOrLauncherDoesntExitCleanly" => "Doesn't always close cleanly",
        "GamepadNotEnabledByDefault" => "The controller must be turned on in the settings",
        "InterfaceTextIsLegible" => "Text is easy to read",
        "InterfaceTextIsNotLegible" => "Some text is small and hard to read",
        "LauncherInteractionIssues" => "Its launcher is hard to use with a controller",
        "NativeResolutionNotDefault" => "Doesn't start at the screen's resolution",
        "NativeResolutionNotSupported" => "Can't run at the screen's resolution",
        "SingleplayerGameplayRequiresActiveInternetConnection" => {
            "Needs an internet connection even to play alone"
        }
        "SteamOSDoesNotSupport" => "Doesn't run on SteamOS",
        "SteamOSDoesNotSupport_VR" => "VR isn't supported on SteamOS",
        "TextInputDoesNotAutomaticallyInvokesKeyboard" => "Bring up the keyboard manually to type",
        "UnsupportedAntiCheatConfiguration" => "Its anti-cheat doesn't run on Steam Deck",
        "UnsupportedAntiCheat_Other" => "Its anti-cheat doesn't run on Steam Deck",
        "UnsupportedGraphicsPerformance" => "Too demanding for the Steam Deck's graphics",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    type Asked = Vec<(String, Vec<(String, String)>)>;

    struct FakeSteam {
        reply: Result<Value, StatusCode>,
        asked: Mutex<Asked>,
    }

    impl FakeSteam {
        fn new(reply: Value) -> FakeSteam {
            FakeSteam {
                reply: Ok(reply),
                asked: Mutex::new(Vec::new()),
            }
        }

        fn failing(status: StatusCode) -> FakeSteam {
            FakeSteam {
                reply: Err(status),
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl Get for FakeSteam {
        fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            _needs_key: bool,
        ) -> impl Future<Output = Result<Value, SteamError>> + Send {
            self.asked.lock().unwrap().push((
                endpoint.to_string(),
                params
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            ));
            std::future::ready(self.reply.clone().map_err(|status| SteamError::Status {
                endpoint: endpoint.to_string(),
                status,
            }))
        }
    }

    fn item(display_type: u32, key: &str) -> Value {
        json!({ "display_type": display_type, "loc_token": format!("{TOKEN_PREFIX}{key}") })
    }

    fn report(items: Vec<Value>) -> Value {
        json!({ "success": 1, "results": { "appid": 570, "resolved_category": 2, "resolved_items": items } })
    }

    fn check(result: CheckResult, text: &'static str) -> DeckCheck {
        DeckCheck { result, text }
    }

    #[tokio::test]
    async fn a_verified_game_has_only_passed_checks() {
        let steam = FakeSteam::new(report(vec![item(4, "DefaultConfigurationIsPerformant")]));

        let checks = get_deck_report(&steam, AppId(620)).await.unwrap();

        assert_eq!(
            checks,
            [check(
                CheckResult::Passed,
                "Runs well with default settings"
            )]
        );
    }

    #[tokio::test]
    async fn a_playable_game_has_warnings_and_notes() {
        let steam = FakeSteam::new(report(vec![
            item(3, "InterfaceTextIsNotLegible"),
            item(3, "ControllerGlyphsDoNotMatchDeckDevice"),
            item(4, "DefaultConfigurationIsPerformant"),
            item(1, "ExternalControllersNotSupportedPrimaryPlayer"),
        ]));

        let checks = get_deck_report(&steam, AppId(570)).await.unwrap();

        let results: Vec<_> = checks.iter().map(|c| c.result).collect();
        assert_eq!(
            results,
            [
                CheckResult::Warning,
                CheckResult::Warning,
                CheckResult::Passed,
                CheckResult::Note
            ]
        );
        assert_eq!(checks[0].text, "Some text is small and hard to read");
        assert_eq!(checks[1].text, "Shows buttons from another controller");
    }

    #[tokio::test]
    async fn an_unsupported_game_says_why() {
        let steam = FakeSteam::new(report(vec![item(2, "UnsupportedAntiCheatConfiguration")]));

        let checks = get_deck_report(&steam, AppId(1_085_660)).await.unwrap();

        assert_eq!(
            checks,
            [check(
                CheckResult::Failed,
                "Its anti-cheat doesn't run on Steam Deck"
            )]
        );
    }

    #[tokio::test]
    async fn an_unknown_key_is_skipped_and_an_unknown_display_type_is_a_note() {
        let steam = FakeSteam::new(report(vec![
            item(4, "ATestThatDoesNotExistYet"),
            item(9, "InterfaceTextIsNotLegible"),
        ]));

        let checks = get_deck_report(&steam, AppId(1)).await.unwrap();

        assert_eq!(
            checks,
            [check(
                CheckResult::Note,
                "Some text is small and hard to read"
            )]
        );
    }

    #[tokio::test]
    async fn a_report_that_did_not_succeed_or_has_no_items_is_empty() {
        let failed = FakeSteam::new(json!({ "success": 0 }));
        let bare = FakeSteam::new(json!({ "success": 1, "results": { "appid": 1 } }));

        assert!(get_deck_report(&failed, AppId(1)).await.unwrap().is_empty());
        assert!(get_deck_report(&bare, AppId(1)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_report_of_the_wrong_shape_is_an_error() {
        let steam = FakeSteam::new(json!({ "success": 1, "results": { "resolved_items": 3 } }));

        let error = get_deck_report(&steam, AppId(1)).await.unwrap_err();

        assert!(matches!(error, SteamError::Shape { .. }));
    }

    #[tokio::test]
    async fn the_report_is_asked_by_appid_in_english() {
        let steam = FakeSteam::new(report(Vec::new()));

        get_deck_report(&steam, AppId(570)).await.unwrap();

        let asked = steam.asked.lock().unwrap();
        assert_eq!(asked[0].0, REPORT_ENDPOINT);
        assert_eq!(
            asked[0].1,
            [
                ("nAppID".to_string(), "570".to_string()),
                ("l".to_string(), "english".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn protondb_gives_tier_and_reports() {
        let steam = FakeSteam::new(json!({
            "tier": "gold", "trendingTier": "platinum", "bestReportedTier": "platinum",
            "score": 0.77, "confidence": "strong", "total": 2797
        }));

        let summary = get_protondb(&steam, AppId(1_091_500)).await.unwrap();

        assert_eq!(
            summary,
            Some(ProtonDb {
                tier: ProtonTier::Gold,
                reports: 2797
            })
        );
        assert_eq!(
            steam.asked.lock().unwrap()[0].0,
            "https://www.protondb.com/api/v1/reports/summaries/1091500.json"
        );
        assert!(steam.asked.lock().unwrap()[0].1.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_protondb_tier_is_pending() {
        let steam = FakeSteam::new(json!({ "tier": "diamond", "total": 3 }));

        let summary = get_protondb(&steam, AppId(1)).await.unwrap().unwrap();

        assert_eq!(summary.tier, ProtonTier::Pending);
    }

    #[tokio::test]
    async fn protondb_without_reports_is_none_and_other_errors_stay() {
        let missing = FakeSteam::failing(StatusCode::NOT_FOUND);
        let broken = FakeSteam::failing(StatusCode::INTERNAL_SERVER_ERROR);

        assert_eq!(get_protondb(&missing, AppId(1)).await.unwrap(), None);
        assert!(matches!(
            get_protondb(&broken, AppId(1)).await,
            Err(SteamError::Status { .. })
        ));
    }
}
