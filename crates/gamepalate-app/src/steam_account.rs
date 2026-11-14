use std::{fmt, fs, path::PathBuf};

use gamepalate_core::ids::SteamId;
use gamepalate_steam::{
    client::{read_persona_name, steam_avatar_file, steam_client_file},
    signals::get_player_summary,
    transport::{Fetch, Get, Transport},
};

use crate::{
    account::{steam_api_key, steam_api_key_ending},
    images::{ImageCache, ImageKind},
    log::failed,
    settings::{self, KeySource},
};

#[derive(Clone, PartialEq, Eq)]
pub struct SteamAccount {
    pub steam_id: SteamId,
    pub name: Option<String>,
    pub avatar: Option<Vec<u8>>,
    pub key: KeySource,
    pub key_ending: Option<String>,
}

impl fmt::Debug for SteamAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SteamAccount")
            .field("name", &self.name)
            .field("avatar_bytes", &self.avatar.as_ref().map(Vec::len))
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct ClientFiles {
    local_config: Option<PathBuf>,
    avatar: Option<PathBuf>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Persona {
    name: Option<String>,
    avatar: Option<Vec<u8>>,
}

impl Persona {
    fn is_complete(&self) -> bool {
        self.name.is_some() && self.avatar.is_some()
    }
}

pub(crate) async fn steam_account<F: Fetch>(
    steam_id: SteamId,
    images: &ImageCache<F>,
) -> SteamAccount {
    let client = ClientFiles {
        local_config: steam_client_file(steam_id),
        avatar: steam_avatar_file(steam_id),
    };
    let mut persona = client_persona(&client);
    if !persona.is_complete()
        && let Some(key) = steam_api_key()
    {
        match Transport::new(Some(key)) {
            Ok(steam) => persona = with_api(persona, steam_id, &steam, images).await,
            Err(error) => failed("could not reach Steam for the account")(&error),
        }
    }

    SteamAccount {
        steam_id,
        name: persona.name,
        avatar: persona.avatar,
        key: settings::key_source(),
        key_ending: steam_api_key_ending(),
    }
}

fn client_persona(client: &ClientFiles) -> Persona {
    let name = client.local_config.as_deref().and_then(|path| {
        read_persona_name(path)
            .inspect_err(failed("could not read the Steam account name"))
            .ok()
            .flatten()
    });
    let avatar = client.avatar.as_deref().and_then(|path| {
        fs::read(path)
            .inspect_err(failed("could not read the Steam account picture"))
            .ok()
    });
    Persona { name, avatar }
}

async fn with_api<S: Get, F: Fetch>(
    mut persona: Persona,
    steam_id: SteamId,
    steam: &S,
    images: &ImageCache<F>,
) -> Persona {
    if persona.is_complete() {
        return persona;
    }
    let Some(profile) = get_player_summary(steam, steam_id)
        .await
        .inspect_err(failed("could not read the Steam account"))
        .ok()
        .flatten()
    else {
        return persona;
    };

    if persona.name.is_none() {
        persona.name = profile.name;
    }
    if persona.avatar.is_none()
        && let Some(url) = profile.avatar_url
    {
        persona.avatar = images
            .image(&url, ImageKind::Extra)
            .await
            .inspect_err(failed("could not download the Steam account picture"))
            .ok();
    }
    persona
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
    };

    use gamepalate_steam::{images::ImageDownloader, transport::SteamError};
    use reqwest::{Request, Response};
    use serde_json::{Value, json};

    use super::*;

    const ID: SteamId = SteamId(76_561_198_000_000_000);
    const AVATAR_URL: &str = "https://avatars.example.com/abc_full.jpg";
    const CLIENT_PICTURE: [u8; 3] = [1, 1, 1];
    const API_PICTURE: [u8; 3] = [2, 2, 2];
    const LOCAL_CONFIG: &str =
        r#""UserLocalConfigStore" { "friends" { "PersonaName" "From the client" } }"#;

    enum Answer {
        Player,
        Nobody,
        Down,
    }

    struct FakeSteam {
        answer: Answer,
        calls: AtomicU32,
    }

    impl FakeSteam {
        fn new(answer: Answer) -> FakeSteam {
            FakeSteam {
                answer,
                calls: AtomicU32::new(0),
            }
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            needs_key: bool,
        ) -> Result<Value, SteamError> {
            assert_eq!(endpoint, "ISteamUser/GetPlayerSummaries/v2");
            assert!(needs_key);
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.answer {
                Answer::Player => Ok(json!({ "response": { "players": [{
                    "steamid": "76561198000000000",
                    "personaname": "From the API",
                    "avatarfull": AVATAR_URL
                }] } })),
                Answer::Nobody => Ok(json!({ "response": { "players": [] } })),
                Answer::Down => Err(SteamError::SteamDown {
                    endpoint: endpoint.to_string(),
                    reason: "timed out".to_string(),
                    source: None,
                }),
            }
        }
    }

    struct FakeCdn {
        calls: Arc<AtomicU32>,
    }

    impl Fetch for FakeCdn {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            assert_eq!(request.url().as_str(), AVATAR_URL);
            self.calls.fetch_add(1, Ordering::Relaxed);
            let response = http::Response::builder()
                .status(200)
                .header("content-type", "image/jpeg")
                .body(API_PICTURE.to_vec())
                .unwrap();
            async move { Ok(response.into()) }
        }
    }

    fn images(dir: &Path) -> (ImageCache<FakeCdn>, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        let cache = ImageCache::new(
            Some(dir.join("images")),
            ImageDownloader::with_fetch(FakeCdn {
                calls: Arc::clone(&calls),
            }),
        );
        (cache, calls)
    }

    fn client(dir: &Path, name: bool, picture: bool) -> ClientFiles {
        let local_config = dir.join("localconfig.vdf");
        let avatar = dir.join("76561198000000000.png");
        fs::write(&local_config, LOCAL_CONFIG).unwrap();
        fs::write(&avatar, CLIENT_PICTURE).unwrap();
        ClientFiles {
            local_config: name.then_some(local_config),
            avatar: picture.then_some(avatar),
        }
    }

    async fn persona(client: &ClientFiles, steam: &FakeSteam, dir: &Path) -> (Persona, u32) {
        let (images, downloads) = images(dir);
        let persona = with_api(client_persona(client), ID, steam, &images).await;
        (persona, downloads.load(Ordering::Relaxed))
    }

    #[tokio::test]
    async fn everything_from_the_steam_client_asks_nothing_to_the_api() {
        let dir = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(Answer::Player);

        let (persona, downloads) =
            persona(&client(dir.path(), true, true), &steam, dir.path()).await;

        assert_eq!(
            persona,
            Persona {
                name: Some("From the client".to_string()),
                avatar: Some(CLIENT_PICTURE.to_vec()),
            }
        );
        assert_eq!(steam.calls(), 0);
        assert_eq!(downloads, 0);
    }

    #[tokio::test]
    async fn the_name_from_the_client_and_the_picture_from_the_api() {
        let dir = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(Answer::Player);

        let (persona, downloads) =
            persona(&client(dir.path(), true, false), &steam, dir.path()).await;

        assert_eq!(
            persona,
            Persona {
                name: Some("From the client".to_string()),
                avatar: Some(API_PICTURE.to_vec()),
            }
        );
        assert_eq!(steam.calls(), 1);
        assert_eq!(downloads, 1);
    }

    #[tokio::test]
    async fn the_picture_from_the_client_and_the_name_from_the_api() {
        let dir = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(Answer::Player);

        let (persona, downloads) =
            persona(&client(dir.path(), false, true), &steam, dir.path()).await;

        assert_eq!(
            persona,
            Persona {
                name: Some("From the API".to_string()),
                avatar: Some(CLIENT_PICTURE.to_vec()),
            }
        );
        assert_eq!(steam.calls(), 1);
        assert_eq!(downloads, 0);
    }

    #[tokio::test]
    async fn the_downloaded_picture_comes_from_the_cache_the_second_time() {
        let dir = tempfile::tempdir().unwrap();
        let steam = FakeSteam::new(Answer::Player);
        let (images, downloads) = images(dir.path());

        for _ in 0..2 {
            let persona = with_api(Persona::default(), ID, &steam, &images).await;
            assert_eq!(persona.avatar, Some(API_PICTURE.to_vec()));
        }

        assert_eq!(downloads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn without_the_steam_client_there_is_nothing() {
        assert_eq!(client_persona(&ClientFiles::default()), Persona::default());
    }

    #[test]
    fn unreadable_client_files_are_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("localconfig.vdf");
        fs::write(&broken, "\"UserLocalConfigStore\" {").unwrap();

        let persona = client_persona(&ClientFiles {
            local_config: Some(broken),
            avatar: Some(dir.path().join("missing.png")),
        });

        assert_eq!(persona, Persona::default());
    }

    #[tokio::test]
    async fn an_api_error_or_no_player_leaves_the_missing_pieces_empty() {
        for answer in [Answer::Down, Answer::Nobody] {
            let dir = tempfile::tempdir().unwrap();
            let steam = FakeSteam::new(answer);

            let (persona, downloads) =
                persona(&client(dir.path(), true, false), &steam, dir.path()).await;

            assert_eq!(persona.name.as_deref(), Some("From the client"));
            assert_eq!(persona.avatar, None);
            assert_eq!(steam.calls(), 1);
            assert_eq!(downloads, 0);
        }
    }

    #[test]
    fn the_debug_text_has_no_picture_bytes_and_no_steam_id() {
        let account = SteamAccount {
            steam_id: ID,
            name: Some("Someone".to_string()),
            avatar: Some(vec![7; 10]),
            key: KeySource::Saved,
            key_ending: Some("3F9A".to_string()),
        };

        let text = format!("{account:?}");

        assert!(text.contains("Some(10)"), "{text}");
        assert!(!text.contains("76561198000000000"), "{text}");
        assert!(!text.contains("3F9A"), "{text}");
    }
}
