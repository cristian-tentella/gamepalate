use std::{collections::HashSet, io, path::Path};

use gamepalate_core::ids::AppId;

use crate::{
    apps::{self, get_app_list},
    archive::{Archive, ArchivedUpdate},
    items::archived_asked,
    transport::SteamError,
};

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("could not read the archive")]
    Archive(#[source] io::Error),

    #[error(transparent)]
    Steam(#[from] SteamError),

    #[error("app {} has no response in the archive", .app.0)]
    Uncovered { app: AppId },
}

pub async fn chain(archive: &Archive) -> Result<Vec<ArchivedUpdate>, ChainError> {
    let mut completed = archive
        .completed()
        .map_err(ChainError::Archive)?
        .into_iter();
    let Some(latest) = completed.next() else {
        return Ok(Vec::new());
    };
    let region = latest.region();
    let mut uncovered: HashSet<AppId> = get_app_list(&latest)
        .await?
        .into_iter()
        .map(|app| app.id)
        .collect();

    covered(&latest, &mut uncovered)?;
    let mut chain = vec![latest];
    while !uncovered.is_empty() {
        match completed.next() {
            Some(older) if older.region() == region => {
                if covered(&older, &mut uncovered)? > 0 {
                    chain.push(older);
                }
            }
            _ => {
                let app = *uncovered.iter().min().expect("some app is uncovered");
                return Err(ChainError::Uncovered { app });
            }
        }
    }
    Ok(chain)
}

fn covered(update: &ArchivedUpdate, uncovered: &mut HashSet<AppId>) -> Result<usize, SteamError> {
    let mut covered = 0;
    for asked in archived_asked(update)? {
        for id in asked? {
            if uncovered.remove(&id) {
                covered += 1;
            }
        }
    }
    Ok(covered)
}

pub async fn rotate(archive: &Archive, kept_updates: u32) -> Result<usize, ChainError> {
    let chain = chain(archive).await?;
    let completed = archive.completed().map_err(ChainError::Archive)?;
    let latest: Vec<&Path> = completed
        .iter()
        .take(kept_updates as usize)
        .map(ArchivedUpdate::dir)
        .collect();

    for update in chain.iter().skip(1) {
        if !latest.contains(&update.dir()) {
            update.remove_responses(apps::ENDPOINT)?;
        }
    }
    let mut kept = latest;
    kept.extend(chain.iter().map(ArchivedUpdate::dir));
    archive.keep_only(&kept).map_err(ChainError::Archive)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use futures::StreamExt;
    use gamepalate_core::{region::Region, timestamp::Timestamp};
    use serde_json::{Value, json};

    use super::*;
    use crate::{items::item_batches, transport::Get};

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const HOUR: i64 = 3_600_000;

    struct FakeSteam {
        listed: Vec<u32>,
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            _params: &[(&str, &str)],
            _needs_key: bool,
        ) -> Result<Value, SteamError> {
            match endpoint {
                "IStoreService/GetAppList/v1" => Ok(json!({ "response": { "apps": self
                    .listed
                    .iter()
                    .map(|appid| json!({
                        "appid": appid,
                        "last_modified": 1,
                        "price_change_number": 1,
                    }))
                    .collect::<Vec<_>>() } })),
                "IStoreBrowseService/GetItems/v1" => Ok(json!({ "response": {} })),
                other => panic!("unexpected request to {other}"),
            }
        }
    }

    async fn completed_update(
        archive: &Archive,
        hours: i64,
        region: Region,
        listed: &[u32],
        asked: &[u32],
    ) -> PathBuf {
        let started_at = Timestamp(NOW.0 + hours * HOUR);
        let update = archive.start_update(started_at, 0, 30.0, region).unwrap();
        let steam = FakeSteam {
            listed: listed.to_vec(),
        };
        let asked: Vec<AppId> = asked.iter().copied().map(AppId).collect();
        {
            let archiving = update.archiving(&steam);
            get_app_list(&archiving).await.unwrap();
            let mut batches = item_batches(&archiving, &asked, "english", region.as_str(), 4);
            while let Some(batch) = batches.next().await {
                batch.unwrap();
            }
        }
        let dir = update.dir().to_path_buf();
        update.complete(started_at).unwrap();
        dir
    }

    async fn chain_dirs(archive: &Archive) -> Vec<PathBuf> {
        chain(archive)
            .await
            .unwrap()
            .iter()
            .map(|update| update.dir().to_path_buf())
            .collect()
    }

    fn folders(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn has_app_list(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("IStoreService_GetAppList_v1-")
        })
    }

    fn us() -> Region {
        Region::new("US").unwrap()
    }

    #[tokio::test]
    async fn an_empty_archive_has_an_empty_chain() {
        let root = tempfile::tempdir().unwrap();

        let chain = chain(&Archive::new(root.path().join("archive")))
            .await
            .unwrap();

        assert!(chain.is_empty());
    }

    #[tokio::test]
    async fn a_full_update_alone_covers_every_app() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2], &[1, 2]).await;
        let full = completed_update(&archive, 1, Region::ITALY, &[1, 2], &[1, 2]).await;

        assert_eq!(chain_dirs(&archive).await, [full]);
    }

    #[tokio::test]
    async fn the_chain_goes_back_through_the_updates_that_answer_a_listed_app() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        let base = completed_update(&archive, 1, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        completed_update(&archive, 2, Region::ITALY, &[1, 2, 3], &[2]).await;
        let latest = completed_update(&archive, 3, Region::ITALY, &[1, 2, 3, 4], &[2, 4]).await;

        assert_eq!(chain_dirs(&archive).await, [latest, base]);
    }

    #[tokio::test]
    async fn the_chain_stops_before_the_full_update_once_every_app_was_renewed() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        let older = completed_update(&archive, 1, Region::ITALY, &[1, 2, 3], &[1, 3]).await;
        let latest = completed_update(&archive, 2, Region::ITALY, &[1, 2, 3], &[2]).await;

        assert_eq!(chain_dirs(&archive).await, [latest, older]);
    }

    #[tokio::test]
    async fn apps_steam_no_longer_lists_do_not_need_a_response() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        let latest = completed_update(&archive, 1, Region::ITALY, &[1, 2], &[1, 2]).await;

        assert_eq!(chain_dirs(&archive).await, [latest]);
    }

    #[tokio::test]
    async fn an_app_without_any_response_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2], &[1, 2]).await;
        completed_update(&archive, 1, Region::ITALY, &[1, 2, 3, 5], &[]).await;

        let error = chain(&archive).await.unwrap_err();

        assert!(matches!(error, ChainError::Uncovered { app: AppId(3) }));
        assert_eq!(error.to_string(), "app 3 has no response in the archive");
    }

    #[tokio::test]
    async fn the_chain_does_not_reach_an_update_of_another_region() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2], &[1, 2]).await;
        completed_update(&archive, 1, us(), &[1, 2], &[1]).await;

        let error = chain(&archive).await.unwrap_err();

        assert!(matches!(error, ChainError::Uncovered { app: AppId(2) }));
    }

    #[tokio::test]
    async fn the_chain_needs_the_app_list_of_the_latest_update() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let latest = completed_update(&archive, 0, Region::ITALY, &[1], &[1]).await;
        archive.completed().unwrap()[0]
            .remove_responses(apps::ENDPOINT)
            .unwrap();

        let error = chain(&archive).await.unwrap_err();

        assert!(matches!(
            error,
            ChainError::Steam(SteamError::NotArchived { dir, .. }) if dir == latest
        ));
    }

    #[tokio::test]
    async fn rotation_keeps_the_latest_completed_updates() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        for hours in 0..4 {
            completed_update(&archive, hours, Region::ITALY, &[1], &[1]).await;
        }

        let removed = rotate(&archive, 3).await.unwrap();

        assert_eq!(removed, 1);
        assert_eq!(folders(root.path()), ["000002", "000003", "000004"]);
    }

    #[tokio::test]
    async fn rotation_removes_failed_interrupted_and_bare_folders() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        archive.start_update(NOW, 0, 30.0, Region::ITALY).unwrap();
        archive
            .start_update(NOW, 0, 30.0, Region::ITALY)
            .unwrap()
            .fail(NOW)
            .unwrap();
        fs::create_dir(root.path().join("000003")).unwrap();
        completed_update(&archive, 0, Region::ITALY, &[1], &[1]).await;
        fs::create_dir(root.path().join("notes")).unwrap();

        rotate(&archive, 3).await.unwrap();

        assert_eq!(folders(root.path()), ["000004", "notes"]);
    }

    #[tokio::test]
    async fn rotation_keeps_what_covers_the_apps_without_its_app_lists() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        let full = completed_update(&archive, 1, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        completed_update(&archive, 2, Region::ITALY, &[1, 2, 3], &[]).await;
        let renewal = completed_update(&archive, 3, Region::ITALY, &[1, 2, 3], &[1]).await;
        let previous = completed_update(&archive, 4, Region::ITALY, &[1, 2, 3], &[]).await;
        let latest = completed_update(&archive, 5, Region::ITALY, &[1, 2, 3], &[2]).await;

        let removed = rotate(&archive, 2).await.unwrap();

        assert_eq!(removed, 2);
        assert_eq!(
            folders(root.path()),
            ["000002", "000004", "000005", "000006"]
        );
        assert!(has_app_list(&latest));
        assert!(has_app_list(&previous));
        assert!(!has_app_list(&renewal));
        assert!(!has_app_list(&full));
        assert_eq!(chain_dirs(&archive).await, [latest, renewal, full]);
    }

    #[tokio::test]
    async fn the_full_update_leaves_once_all_its_apps_were_renewed() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1, 2, 3], &[1, 2, 3]).await;
        completed_update(&archive, 1, Region::ITALY, &[1, 2, 3], &[1]).await;
        rotate(&archive, 1).await.unwrap();
        assert_eq!(folders(root.path()), ["000001", "000002"]);

        completed_update(&archive, 2, Region::ITALY, &[1, 2, 3], &[2, 3]).await;
        rotate(&archive, 1).await.unwrap();

        assert_eq!(folders(root.path()), ["000002", "000003"]);
    }

    #[tokio::test]
    async fn rotation_removes_nothing_when_an_app_is_uncovered() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, 0, Region::ITALY, &[1], &[1]).await;
        completed_update(&archive, 1, Region::ITALY, &[1], &[1]).await;
        completed_update(&archive, 2, Region::ITALY, &[1, 2], &[]).await;

        let error = rotate(&archive, 1).await.unwrap_err();

        assert!(matches!(error, ChainError::Uncovered { app: AppId(2) }));
        assert_eq!(folders(root.path()), ["000001", "000002", "000003"]);
    }
}
