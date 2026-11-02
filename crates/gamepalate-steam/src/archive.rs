use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use gamepalate_core::{region::Region, timestamp::Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::transport::{Get, SteamError};

const MANIFEST_FILE: &str = "update.json";
const RESPONSE_SUFFIX: &str = ".json.gz";
const FOLDER_DIGITS: usize = 6;
const HASH_LENGTH: usize = 20;

pub struct Archive {
    root: PathBuf,
}

pub struct UpdateFolder {
    dir: PathBuf,
    manifest: Manifest,
    started_at: Timestamp,
    resumed_responses: usize,
}

pub struct ArchivedUpdate {
    dir: PathBuf,
    started_at: Timestamp,
    full: bool,
    region: Region,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingUpdate {
    pub started_at: Timestamp,
    pub responses: usize,
}

pub struct Archiving<'a, S> {
    steam: &'a S,
    dir: &'a Path,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    started_at: String,
    full: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failed_at: Option<String>,
}

#[derive(Serialize)]
struct RecordToWrite<'a> {
    endpoint: &'a str,
    params: &'a Map<String, Value>,
    body: &'a Value,
}

#[derive(Deserialize)]
pub struct ArchivedRecord {
    #[serde(default)]
    pub params: Map<String, Value>,
    pub body: Value,
}

impl Archive {
    pub fn new(root: PathBuf) -> Archive {
        Archive { root }
    }

    pub fn start_update(
        &self,
        now: Timestamp,
        resume_within_hours: u32,
        tags_max_age_days: f64,
        region: Region,
    ) -> io::Result<UpdateFolder> {
        if let Some((dir, manifest, pending)) = self.resumable(now, resume_within_hours, region)? {
            return Ok(UpdateFolder {
                dir,
                manifest,
                started_at: pending.started_at,
                resumed_responses: pending.responses,
            });
        }

        fs::create_dir_all(&self.root)?;
        let last = self.folder_numbers()?.into_iter().max().unwrap_or(0);
        let dir = self.root.join(format!("{:0FOLDER_DIGITS$}", last + 1));
        fs::create_dir(&dir)?;

        let full = match self.chain()?.last() {
            Some(base) if base.full && base.region == region => {
                now.ms_since(base.started_at) as f64 >= tags_max_age_days * 86_400_000.0
            }
            _ => true,
        };
        let manifest = Manifest {
            started_at: iso(now),
            full,
            region: Some(region.to_string()),
            completed_at: None,
            failed_at: None,
        };
        write_manifest(&dir, &manifest)?;
        Ok(UpdateFolder {
            dir,
            manifest,
            started_at: now,
            resumed_responses: 0,
        })
    }

    pub fn chain(&self) -> io::Result<Vec<ArchivedUpdate>> {
        let mut chain = Vec::new();
        for (dir, manifest) in self.updates_newest_first()? {
            if manifest.completed_at.is_none() {
                continue;
            }
            let full = manifest.full;
            chain.push(ArchivedUpdate {
                started_at: parse_iso(&manifest.started_at)?,
                region: manifest.region()?,
                dir,
                full,
            });
            if full {
                break;
            }
        }
        Ok(chain)
    }

    pub fn rotate(&self, kept_updates: u32) -> io::Result<usize> {
        let mut updates = self.updates_newest_first()?;
        updates.reverse();
        let completed: Vec<&(PathBuf, Manifest)> = updates
            .iter()
            .filter(|(_, manifest)| manifest.completed_at.is_some())
            .collect();

        let mut first = completed.len().saturating_sub(kept_updates as usize);
        while first > 0 && !completed[first].1.full {
            first -= 1;
        }
        let kept: Vec<&Path> = completed[first..]
            .iter()
            .map(|(dir, _)| dir.as_path())
            .collect();

        let mut removed = 0;
        for number in self.folder_numbers()? {
            let dir = self.root.join(format!("{number:0FOLDER_DIGITS$}"));
            if !kept.contains(&dir.as_path()) {
                fs::remove_dir_all(&dir)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn pending_update(
        &self,
        now: Timestamp,
        resume_within_hours: u32,
        region: Region,
    ) -> io::Result<Option<PendingUpdate>> {
        Ok(self
            .resumable(now, resume_within_hours, region)?
            .map(|(_, _, pending)| pending))
    }

    fn resumable(
        &self,
        now: Timestamp,
        resume_within_hours: u32,
        region: Region,
    ) -> io::Result<Option<(PathBuf, Manifest, PendingUpdate)>> {
        let Some((dir, manifest)) = self.updates_newest_first()?.into_iter().next() else {
            return Ok(None);
        };
        let started_at = parse_iso(&manifest.started_at)?;
        let recent = now.ms_since(started_at) < i64::from(resume_within_hours) * 3_600_000;
        let interrupted = manifest.completed_at.is_none() && manifest.failed_at.is_none();
        if !(interrupted && recent && manifest.region()? == region) {
            return Ok(None);
        }

        let responses = count_responses(&dir)?;
        Ok(Some((
            dir,
            manifest,
            PendingUpdate {
                started_at,
                responses,
            },
        )))
    }

    fn updates_newest_first(&self) -> io::Result<Vec<(PathBuf, Manifest)>> {
        let mut numbers = match self.folder_numbers() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            numbers => numbers?,
        };
        numbers.sort_unstable_by(|a, b| b.cmp(a));

        let mut updates = Vec::new();
        for number in numbers {
            let dir = self.root.join(format!("{number:0FOLDER_DIGITS$}"));
            if let Some(manifest) = read_manifest(&dir)? {
                updates.push((dir, manifest));
            }
        }
        Ok(updates)
    }

    fn folder_numbers(&self) -> io::Result<Vec<u32>> {
        let mut numbers = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() == FOLDER_DIGITS
                && let Ok(number) = name.parse()
            {
                numbers.push(number);
            }
        }
        Ok(numbers)
    }
}

impl Manifest {
    fn region(&self) -> io::Result<Region> {
        match &self.region {
            None => Ok(Region::ITALY),
            Some(code) => Region::new(code).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the update has an invalid region {code:?}"),
                )
            }),
        }
    }
}

impl UpdateFolder {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn archiving<'a, S: Get>(&'a self, steam: &'a S) -> Archiving<'a, S> {
        Archiving {
            steam,
            dir: &self.dir,
        }
    }

    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub fn resumed_responses(&self) -> usize {
        self.resumed_responses
    }

    pub fn full(&self) -> bool {
        self.manifest.full
    }

    pub fn complete(mut self, now: Timestamp) -> io::Result<()> {
        self.manifest.completed_at = Some(iso(now));
        write_manifest(&self.dir, &self.manifest)
    }

    pub fn fail(mut self, now: Timestamp) -> io::Result<()> {
        self.manifest.failed_at = Some(iso(now));
        write_manifest(&self.dir, &self.manifest)
    }
}

impl ArchivedUpdate {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn region(&self) -> Region {
        self.region
    }

    pub fn started_at(&self) -> Timestamp {
        self.started_at
    }

    pub fn full(&self) -> bool {
        self.full
    }

    pub fn records(
        &self,
        endpoint: &str,
    ) -> Result<impl Iterator<Item = Result<ArchivedRecord, SteamError>>, SteamError> {
        let archive_error = |source| SteamError::Archive {
            path: self.dir.clone(),
            source,
        };
        let prefix = format!("{}-", file_endpoint(endpoint));
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(archive_error)? {
            let name = entry.map_err(archive_error)?.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(RESPONSE_SUFFIX) {
                paths.push(self.dir.join(&*name));
            }
        }
        paths.sort();

        Ok(paths
            .into_iter()
            .map(|path| read_record(&path).map_err(|source| SteamError::Archive { path, source })))
    }
}

impl Get for ArchivedUpdate {
    async fn get(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
        _needs_key: bool,
    ) -> Result<Value, SteamError> {
        let path = self.dir.join(response_file(endpoint, &canonical(params)));
        if !path.exists() {
            return Err(SteamError::NotArchived {
                endpoint: endpoint.to_string(),
                dir: self.dir.clone(),
            });
        }
        read_record(&path)
            .map(|record| record.body)
            .map_err(|source| SteamError::Archive { path, source })
    }
}

impl<S: Get + Sync> Get for Archiving<'_, S> {
    async fn get(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
        needs_key: bool,
    ) -> Result<Value, SteamError> {
        let canonical_params = canonical(params);
        let path = self.dir.join(response_file(endpoint, &canonical_params));
        let archive_error = |source| SteamError::Archive {
            path: path.clone(),
            source,
        };

        if path.exists() {
            return read_record(&path)
                .map(|record| record.body)
                .map_err(archive_error);
        }

        let body = self.steam.get(endpoint, params, needs_key).await?;
        let record = RecordToWrite {
            endpoint,
            params: &canonical_params,
            body: &body,
        };
        write_record(&path, &record).map_err(archive_error)?;
        Ok(body)
    }
}

fn canonical(params: &[(&str, &str)]) -> Map<String, Value> {
    params
        .iter()
        .map(|(name, value)| (name.to_string(), Value::String(value.to_string())))
        .collect()
}

fn response_file(endpoint: &str, params: &Map<String, Value>) -> String {
    let key = json!([endpoint, params]).to_string();
    let hash: String = Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "{}-{}{RESPONSE_SUFFIX}",
        file_endpoint(endpoint),
        &hash[..HASH_LENGTH]
    )
}

fn file_endpoint(endpoint: &str) -> String {
    endpoint.replace('/', "_")
}

fn read_record(path: &Path) -> io::Result<ArchivedRecord> {
    let mut text = String::new();
    GzDecoder::new(fs::File::open(path)?).read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

fn write_record(path: &Path, record: &RecordToWrite) -> io::Result<()> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut encoder, record)?;
    write_atomically(path, &encoder.finish()?)
}

fn count_responses(dir: &Path) -> io::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        if entry?
            .file_name()
            .to_string_lossy()
            .ends_with(RESPONSE_SUFFIX)
        {
            count += 1;
        }
    }
    Ok(count)
}

fn read_manifest(dir: &Path) -> io::Result<Option<Manifest>> {
    let path = dir.join(MANIFEST_FILE);
    let text = match fs::read(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        text => text?,
    };
    serde_json::from_slice(&text).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> io::Result<()> {
    let text = serde_json::to_vec_pretty(manifest)?;
    write_atomically(&dir.join(MANIFEST_FILE), &text)
}

fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut temporary = OsString::from(path.as_os_str());
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let mut file = fs::File::create(&temporary)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

fn iso(timestamp: Timestamp) -> String {
    jiff::Timestamp::from_millisecond(timestamp.0)
        .expect("a timestamp within jiff's range")
        .to_string()
}

fn parse_iso(text: &str) -> io::Result<Timestamp> {
    text.parse::<jiff::Timestamp>()
        .map(|timestamp| Timestamp(timestamp.as_millisecond()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const TAG_LIST: &str = "IStoreService/GetTagList/v1";
    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    type Asked = (String, Vec<(String, String)>, bool);

    struct FakeSteam {
        asked: Mutex<Vec<Asked>>,
        fails: bool,
    }

    impl FakeSteam {
        fn new() -> FakeSteam {
            FakeSteam {
                asked: Mutex::new(Vec::new()),
                fails: false,
            }
        }

        fn asked(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
    }

    impl Get for FakeSteam {
        async fn get(
            &self,
            endpoint: &str,
            params: &[(&str, &str)],
            needs_key: bool,
        ) -> Result<Value, SteamError> {
            let params = params
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            self.asked
                .lock()
                .unwrap()
                .push((endpoint.to_string(), params, needs_key));
            if self.fails {
                return Err(SteamError::Status {
                    endpoint: endpoint.to_string(),
                    status: reqwest::StatusCode::NOT_FOUND,
                });
            }
            Ok(json!({ "response": { "asked": self.asked() } }))
        }
    }

    fn archived_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(RESPONSE_SUFFIX))
            .collect();
        names.sort();
        names
    }

    fn read_record_json(path: &Path) -> Value {
        let mut text = String::new();
        GzDecoder::new(fs::File::open(path).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn read_manifest(dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn the_first_answer_is_archived_and_the_second_comes_from_the_file() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let steam = FakeSteam::new();
        let archiving = update.archiving(&steam);

        let first = archiving
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();
        let second = archiving
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();

        assert_eq!(steam.asked(), 1);
        assert_eq!(first, second);
        assert_eq!(archived_files(update.dir()).len(), 1);
    }

    #[tokio::test]
    async fn the_file_holds_endpoint_params_and_body() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let steam = FakeSteam::new();

        update
            .archiving(&steam)
            .get(TAG_LIST, &[("language", "italian")], false)
            .await
            .unwrap();

        let files = archived_files(update.dir());
        assert_eq!(
            read_record_json(&update.dir().join(&files[0])),
            json!({
                "endpoint": TAG_LIST,
                "params": { "language": "italian" },
                "body": { "response": { "asked": 1 } },
            })
        );
    }

    #[tokio::test]
    async fn the_request_reaches_steam_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let steam = FakeSteam::new();

        update
            .archiving(&steam)
            .get(
                "IStoreService/GetAppList/v1",
                &[("max_results", "50000")],
                true,
            )
            .await
            .unwrap();

        assert_eq!(
            steam.asked.lock().unwrap()[0],
            (
                "IStoreService/GetAppList/v1".to_string(),
                vec![("max_results".to_string(), "50000".to_string())],
                true
            )
        );
    }

    #[test]
    fn the_file_name_is_the_endpoint_and_twenty_hex_digits() {
        let name = response_file(TAG_LIST, &canonical(&[("language", "english")]));

        let hash = name
            .strip_prefix("IStoreService_GetTagList_v1-")
            .and_then(|rest| rest.strip_suffix(RESPONSE_SUFFIX))
            .unwrap();
        assert_eq!(hash.len(), HASH_LENGTH);
        assert!(hash.chars().all(|char| char.is_ascii_hexdigit()));
    }

    #[test]
    fn the_order_of_the_params_does_not_change_the_file() {
        let one = response_file(TAG_LIST, &canonical(&[("a", "1"), ("b", "2")]));
        let other = response_file(TAG_LIST, &canonical(&[("b", "2"), ("a", "1")]));

        assert_eq!(one, other);
    }

    #[test]
    fn different_params_or_endpoints_give_different_files() {
        let english = response_file(TAG_LIST, &canonical(&[("language", "english")]));
        let italian = response_file(TAG_LIST, &canonical(&[("language", "italian")]));
        let other = response_file(
            "IStoreService/Other/v1",
            &canonical(&[("language", "english")]),
        );

        assert_ne!(english, italian);
        assert_ne!(english[english.len() - 28..], other[other.len() - 28..]);
    }

    #[test]
    fn the_file_name_stays_the_same_across_versions() {
        assert_eq!(
            response_file(TAG_LIST, &canonical(&[("language", "english")])),
            "IStoreService_GetTagList_v1-08a08165d3070bb26c18.json.gz"
        );
    }

    #[tokio::test]
    async fn a_failed_request_archives_nothing() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let steam = FakeSteam {
            fails: true,
            ..FakeSteam::new()
        };

        let result = update
            .archiving(&steam)
            .get(TAG_LIST, &[("language", "english")], false)
            .await;

        assert!(matches!(result, Err(SteamError::Status { .. })));
        assert!(archived_files(update.dir()).is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_file_is_an_archive_error() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let params = canonical(&[("language", "english")]);
        fs::write(
            update.dir().join(response_file(TAG_LIST, &params)),
            "not gzip",
        )
        .unwrap();
        let steam = FakeSteam::new();

        let result = update
            .archiving(&steam)
            .get(TAG_LIST, &[("language", "english")], false)
            .await;

        assert!(matches!(result, Err(SteamError::Archive { .. })));
        assert_eq!(steam.asked(), 0);
    }

    #[tokio::test]
    async fn an_interrupted_update_is_pending_with_its_responses() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().join("archive"));
        assert_eq!(
            archive.pending_update(NOW, 24, Region::ITALY).unwrap(),
            None
        );
        let update = archive.start_update(NOW, 24, 0.0, Region::ITALY).unwrap();
        update
            .archiving(&FakeSteam::new())
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();

        let pending = archive.pending_update(NOW, 24, Region::ITALY).unwrap();

        assert_eq!(
            pending,
            Some(PendingUpdate {
                started_at: NOW,
                responses: 1,
            })
        );
        let resumed = archive.start_update(NOW, 24, 0.0, Region::ITALY).unwrap();
        assert_eq!(resumed.dir(), update.dir());
        assert_eq!(resumed.resumed_responses(), 1);
    }

    #[test]
    fn a_finished_old_or_foreign_update_is_not_pending() {
        let later = Timestamp(NOW.0 + 25 * 3_600_000);
        let us = Region::new("US").unwrap();
        type Finish = fn(UpdateFolder);
        let cases: [(&str, Finish); 3] = [
            ("completed", |update| update.complete(NOW).unwrap()),
            ("failed", |update| update.fail(NOW).unwrap()),
            ("interrupted", |_| {}),
        ];

        for (name, finish) in cases {
            let root = tempfile::tempdir().unwrap();
            let archive = Archive::new(root.path().join("archive"));
            finish(archive.start_update(NOW, 24, 0.0, Region::ITALY).unwrap());

            let pending_now = archive.pending_update(NOW, 24, Region::ITALY).unwrap();
            assert_eq!(pending_now.is_some(), name == "interrupted", "{name}");
            assert_eq!(
                archive.pending_update(later, 24, Region::ITALY).unwrap(),
                None
            );
            assert_eq!(archive.pending_update(NOW, 24, us).unwrap(), None);

            let next = archive.start_update(NOW, 24, 0.0, Region::ITALY).unwrap();
            assert_eq!(
                next.dir().ends_with("000001"),
                pending_now.is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn every_update_gets_the_next_numbered_folder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("000007")).unwrap();
        fs::create_dir(root.path().join("notes")).unwrap();
        let archive = Archive::new(root.path().to_path_buf());

        let first = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        let second = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();

        assert_eq!(first.dir(), root.path().join("000008"));
        assert_eq!(second.dir(), root.path().join("000009"));
    }

    #[test]
    fn a_new_update_is_full_and_not_completed() {
        let root = tempfile::tempdir().unwrap();

        let update = Archive::new(root.path().join("archive"))
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();

        assert_eq!(
            read_manifest(update.dir()),
            json!({ "startedAt": "2026-11-02T17:00:00Z", "full": true, "region": "IT" })
        );
    }

    #[test]
    fn completing_records_when_it_ended() {
        let root = tempfile::tempdir().unwrap();
        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap();
        let dir = update.dir().to_path_buf();

        update.complete(Timestamp(NOW.0 + 90_000)).unwrap();

        assert_eq!(
            read_manifest(&dir),
            json!({
                "startedAt": "2026-11-02T17:00:00Z",
                "full": true,
                "region": "IT",
                "completedAt": "2026-11-02T17:01:30Z",
            })
        );
        assert!(!dir.join("update.json.tmp").exists());
    }

    fn latest(archive: &Archive) -> ArchivedUpdate {
        archive.chain().unwrap().into_iter().next().unwrap()
    }

    fn completed_update(archive: &Archive, started_at: Timestamp) -> PathBuf {
        let update = archive
            .start_update(started_at, 0, 0.0, Region::ITALY)
            .unwrap();
        let dir = update.dir().to_path_buf();
        update.complete(started_at).unwrap();
        dir
    }

    #[test]
    fn an_archive_that_does_not_exist_has_no_completed_update() {
        let root = tempfile::tempdir().unwrap();

        let chain = Archive::new(root.path().join("archive")).chain().unwrap();

        assert!(chain.is_empty());
    }

    #[test]
    fn the_latest_completed_skips_unfinished_updates_and_bare_folders() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, Timestamp(NOW.0 - 60_000));
        let expected = completed_update(&archive, NOW);
        archive
            .start_update(Timestamp(NOW.0 + 60_000), 0, 0.0, Region::ITALY)
            .unwrap();
        fs::create_dir(root.path().join("000009")).unwrap();

        let latest = latest(&archive);

        assert_eq!(latest.dir(), expected);
        assert_eq!(latest.started_at(), NOW);
    }

    #[test]
    fn only_unfinished_updates_means_no_completed_update() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();

        assert!(archive.chain().unwrap().is_empty());
    }

    #[test]
    fn a_broken_manifest_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let update = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        fs::write(update.dir().join(MANIFEST_FILE), "{").unwrap();

        let error = archive.chain().err().unwrap();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_completed_update_answers_what_it_archived_without_steam() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let update = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        let steam = FakeSteam::new();
        let live = update
            .archiving(&steam)
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();
        update.complete(NOW).unwrap();

        let archived = latest(&archive);
        let replayed = archived
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();

        assert_eq!(replayed, live);
        assert_eq!(steam.asked(), 1);
    }

    #[tokio::test]
    async fn a_request_the_update_never_made_is_not_archived() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);

        let result = latest(&archive)
            .get(TAG_LIST, &[("language", "italian")], false)
            .await;

        match result {
            Err(error @ SteamError::NotArchived { .. }) => {
                assert!(error.to_string().ends_with(TAG_LIST))
            }
            other => panic!("expected NotArchived, got {other:?}"),
        }
    }

    const HOUR: i64 = 3_600_000;

    async fn interrupted_update(archive: &Archive, responses: &[&str]) -> PathBuf {
        let update = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        let steam = FakeSteam::new();
        for language in responses {
            update
                .archiving(&steam)
                .get(TAG_LIST, &[("language", language)], false)
                .await
                .unwrap();
        }
        update.dir().to_path_buf()
    }

    #[tokio::test]
    async fn an_interrupted_update_resumes_with_its_start_and_responses() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = interrupted_update(&archive, &["english", "italian"]).await;

        let resumed = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();

        assert_eq!(resumed.dir(), dir);
        assert_eq!(resumed.started_at(), NOW);
        assert_eq!(resumed.resumed_responses(), 2);
    }

    #[tokio::test]
    async fn a_resumed_update_does_not_ask_steam_again() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        interrupted_update(&archive, &["english"]).await;
        let resumed = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();
        let steam = FakeSteam::new();

        let archiving = resumed.archiving(&steam);
        archiving
            .get(TAG_LIST, &[("language", "english")], false)
            .await
            .unwrap();
        archiving
            .get(TAG_LIST, &[("language", "italian")], false)
            .await
            .unwrap();

        assert_eq!(steam.asked(), 1);
        assert_eq!(steam.asked.lock().unwrap()[0].1[0].1, "italian");
    }

    #[tokio::test]
    async fn an_update_too_old_to_resume_starts_a_new_folder() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = interrupted_update(&archive, &["english"]).await;
        let later = Timestamp(NOW.0 + 24 * HOUR);

        let update = archive.start_update(later, 24, 0.0, Region::ITALY).unwrap();

        assert_ne!(update.dir(), dir);
        assert_eq!(update.started_at(), later);
        assert_eq!(update.resumed_responses(), 0);
    }

    #[tokio::test]
    async fn a_failed_update_is_not_resumed() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let update = archive.start_update(NOW, 24, 0.0, Region::ITALY).unwrap();
        let dir = update.dir().to_path_buf();
        update.fail(Timestamp(NOW.0 + 60_000)).unwrap();

        let next = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();

        assert_ne!(next.dir(), dir);
        assert_eq!(read_manifest(&dir)["failedAt"], "2026-11-02T17:01:00Z");
    }

    #[test]
    fn a_completed_update_is_not_resumed() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = completed_update(&archive, NOW);

        let next = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();

        assert_ne!(next.dir(), dir);
    }

    #[tokio::test]
    async fn only_the_latest_update_can_resume() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        interrupted_update(&archive, &["english"]).await;
        let failed = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        failed.fail(NOW).unwrap();

        let next = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();

        assert_eq!(next.dir(), root.path().join("000003"));
    }

    fn folders(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn make_incremental(dir: &Path) {
        let mut manifest = read_manifest(dir);
        manifest["full"] = json!(false);
        fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();
    }

    #[test]
    fn rotation_keeps_the_latest_completed_updates() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        for _ in 0..4 {
            completed_update(&archive, NOW);
        }

        let removed = archive.rotate(3).unwrap();

        assert_eq!(removed, 1);
        assert_eq!(folders(root.path()), ["000002", "000003", "000004"]);
    }

    #[test]
    fn rotation_removes_failed_interrupted_and_bare_folders() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        archive
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap()
            .fail(NOW)
            .unwrap();
        fs::create_dir(root.path().join("000003")).unwrap();
        completed_update(&archive, NOW);
        fs::create_dir(root.path().join("notes")).unwrap();

        archive.rotate(3).unwrap();

        assert_eq!(folders(root.path()), ["000004", "notes"]);
    }

    #[test]
    fn rotation_keeps_the_full_update_the_kept_ones_build_on() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);
        completed_update(&archive, NOW);
        for _ in 0..3 {
            make_incremental(&completed_update(&archive, NOW));
        }

        archive.rotate(2).unwrap();

        assert_eq!(
            folders(root.path()),
            ["000002", "000003", "000004", "000005"]
        );
    }

    #[test]
    fn rotation_of_an_archive_without_completed_updates_removes_them_all() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();

        archive.rotate(3).unwrap();

        assert!(folders(root.path()).is_empty());
    }

    const DAY: i64 = 24 * HOUR;

    #[test]
    fn the_first_update_is_full() {
        let root = tempfile::tempdir().unwrap();

        let update = Archive::new(root.path().to_path_buf())
            .start_update(NOW, 24, 7.0, Region::ITALY)
            .unwrap();

        assert!(update.full());
    }

    #[test]
    fn an_update_after_a_recent_full_one_is_incremental() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);

        let update = archive
            .start_update(Timestamp(NOW.0 + DAY), 24, 7.0, Region::ITALY)
            .unwrap();

        assert!(!update.full());
        assert_eq!(read_manifest(update.dir())["full"], false);
    }

    #[test]
    fn an_update_is_full_again_once_the_tags_are_old() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);
        make_incremental(&completed_update(&archive, Timestamp(NOW.0 + DAY)));

        let update = archive
            .start_update(Timestamp(NOW.0 + 7 * DAY), 24, 7.0, Region::ITALY)
            .unwrap();

        assert!(update.full());
    }

    fn us() -> Region {
        Region::new("US").unwrap()
    }

    fn without_region(dir: &Path) {
        let mut manifest = read_manifest(dir);
        manifest.as_object_mut().unwrap().remove("region");
        fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();
    }

    #[test]
    fn an_update_without_region_is_italian() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        without_region(&completed_update(&archive, NOW));

        assert_eq!(archive.chain().unwrap()[0].region(), Region::ITALY);
        let update = archive
            .start_update(Timestamp(NOW.0 + DAY), 24, 7.0, Region::ITALY)
            .unwrap();
        assert!(!update.full());
    }

    #[test]
    fn an_update_in_another_region_is_full() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);

        let update = archive
            .start_update(Timestamp(NOW.0 + DAY), 24, 7.0, us())
            .unwrap();

        assert!(update.full());
        assert_eq!(read_manifest(update.dir())["region"], "US");
    }

    #[test]
    fn an_invalid_region_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = completed_update(&archive, NOW);
        let mut manifest = read_manifest(&dir);
        manifest["region"] = json!("usa");
        fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();

        let error = archive.chain().err().unwrap();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn an_interrupted_update_of_another_region_is_not_resumed() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = interrupted_update(&archive, &["english"]).await;

        let update = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, us())
            .unwrap();
        assert_ne!(update.dir(), dir);
        assert_eq!(update.resumed_responses(), 0);
    }

    #[tokio::test]
    async fn an_interrupted_update_without_region_resumes_in_italy() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let dir = interrupted_update(&archive, &["english"]).await;
        without_region(&dir);

        let resumed = archive
            .start_update(Timestamp(NOW.0 + HOUR), 24, 0.0, Region::ITALY)
            .unwrap();

        assert_eq!(resumed.dir(), dir);
    }

    #[test]
    fn the_chain_goes_back_to_the_last_full_update() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        completed_update(&archive, NOW);
        let base = completed_update(&archive, Timestamp(NOW.0 + HOUR));
        let middle = completed_update(&archive, Timestamp(NOW.0 + 2 * HOUR));
        make_incremental(&middle);
        archive
            .start_update(NOW, 0, 0.0, Region::ITALY)
            .unwrap()
            .fail(NOW)
            .unwrap();
        let latest = completed_update(&archive, Timestamp(NOW.0 + 3 * HOUR));
        make_incremental(&latest);

        let chain = archive.chain().unwrap();

        let dirs: Vec<&Path> = chain.iter().map(ArchivedUpdate::dir).collect();
        assert_eq!(dirs, [latest.as_path(), middle.as_path(), base.as_path()]);
        assert_eq!(
            chain.iter().map(ArchivedUpdate::full).collect::<Vec<_>>(),
            [false, false, true]
        );
    }

    #[tokio::test]
    async fn records_are_the_responses_of_one_endpoint_with_their_params() {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::new(root.path().to_path_buf());
        let update = archive.start_update(NOW, 0, 0.0, Region::ITALY).unwrap();
        let steam = FakeSteam::new();
        let archiving = update.archiving(&steam);
        for language in ["english", "italian"] {
            archiving
                .get(TAG_LIST, &[("language", language)], false)
                .await
                .unwrap();
        }
        archiving
            .get("IStoreService/GetAppList/v1", &[], true)
            .await
            .unwrap();
        update.complete(NOW).unwrap();

        let mut languages: Vec<Value> = latest(&archive)
            .records(TAG_LIST)
            .unwrap()
            .map(|record| record.unwrap().params["language"].clone())
            .collect();
        languages.sort_by_key(Value::to_string);

        assert_eq!(languages, [json!("english"), json!("italian")]);
    }
}
