use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Duration;

use gamepalate_app::trailer::{Segments, TrailerDownloader, TrailerError};
use tokio::runtime::Handle;

const AHEAD: usize = 2;
const ATTEMPTS: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(300);

pub fn find_box<'a>(bytes: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let (first, rest) = path.split_first()?;
    let mut offset = 0;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let (header, size) = match size {
            0 => (8, bytes.len() - offset),
            1 => {
                let large = bytes.get(offset + 8..offset + 16)?;
                (16, u64::from_be_bytes(large.try_into().ok()?) as usize)
            }
            size => (8, size),
        };
        let end = offset.checked_add(size)?;
        if size < header {
            return None;
        }
        let body = bytes.get(offset + header..end)?;
        if kind == *first {
            return match rest {
                [] => Some(body),
                rest => find_box(body, rest),
            };
        }
        offset = end;
    }

    None
}

pub fn tfdt(segment: &[u8]) -> Option<i64> {
    let body = find_box(segment, &[b"moof", b"traf", b"tfdt"])?;
    let time = match body.first()? {
        1 => u64::from_be_bytes(body.get(4..12)?.try_into().ok()?),
        _ => u64::from(u32::from_be_bytes(body.get(4..8)?.try_into().ok()?)),
    };

    i64::try_from(time).ok()
}

pub fn media_time(init: &[u8]) -> i64 {
    let Some(body) = find_box(init, &[b"moov", b"trak", b"edts", b"elst"]) else {
        return 0;
    };
    let entries = body
        .get(4..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
        .unwrap_or(0);
    if entries == 0 {
        return 0;
    }
    let time = match body.first() {
        Some(1) => body
            .get(16..24)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i64::from_be_bytes),
        _ => body
            .get(12..16)
            .and_then(|bytes| bytes.try_into().ok())
            .map(|bytes| i64::from(i32::from_be_bytes(bytes))),
    };

    time.unwrap_or(0).max(0)
}

pub fn timescale(init: &[u8]) -> Option<u64> {
    let body = find_box(init, &[b"moov", b"trak", b"mdia", b"mdhd"])?;
    let at = match body.first()? {
        1 => 20,
        _ => 12,
    };
    let timescale = u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?);

    (timescale > 0).then_some(u64::from(timescale))
}

pub fn sample_time(
    tfdt: i64,
    first: i64,
    composition: i64,
    media_time: i64,
    timescale: u64,
) -> f64 {
    (tfdt + composition - first - media_time) as f64 / timescale as f64
}

pub struct Timeline {
    start: u32,
    last: u32,
    duration: f64,
}

impl Timeline {
    pub fn new(segments: &Segments) -> Timeline {
        Timeline {
            start: segments.start_number,
            last: segments.last(),
            duration: segments.nominal_duration().as_secs_f64(),
        }
    }

    pub fn learn(&mut self, number: u32, start: f64) {
        if number > self.start && start > 0.0 {
            self.duration = start / f64::from(number - self.start);
        }
    }

    pub fn segment_at(&self, time: f64) -> u32 {
        let index = (time.max(0.0) / self.duration).floor();
        let index = if index >= f64::from(self.last - self.start) {
            self.last - self.start
        } else {
            index as u32
        };

        self.start + index
    }

    pub fn segment_before(&self, time: f64) -> u32 {
        self.segment_at(time).saturating_sub(1).max(self.start)
    }
}

pub fn download<F>(runtime: &Handle, request: impl Fn() -> F) -> Result<Vec<u8>, String>
where
    F: Future<Output = Result<Vec<u8>, TrailerError>>,
{
    let mut attempt = 1;
    loop {
        match runtime.block_on(request()) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if attempt >= ATTEMPTS => return Err(error.to_string()),
            Err(error) => {
                tracing::debug!("trailer download, attempt {attempt}: {error}");
                attempt += 1;
                std::thread::sleep(RETRY_PAUSE);
            }
        }
    }
}

pub enum Next {
    Segment(u32, Vec<u8>),
    Waiting,
    Finished,
    Failed(String),
}

pub struct Fetcher {
    receiver: Receiver<Result<(u32, Vec<u8>), String>>,
}

impl Fetcher {
    pub fn start(
        runtime: Handle,
        downloader: Arc<TrailerDownloader>,
        segments: Segments,
        first: u32,
    ) -> Result<Fetcher, String> {
        let (sender, receiver) = sync_channel(AHEAD);
        std::thread::Builder::new()
            .name("trailer-download".to_string())
            .spawn(move || fetch(&runtime, &downloader, &segments, first, &sender))
            .map_err(|error| format!("could not start the trailer download: {error}"))?;

        Ok(Fetcher { receiver })
    }

    pub fn next(&self, wait: Duration) -> Next {
        match self.receiver.recv_timeout(wait) {
            Ok(Ok((number, bytes))) => Next::Segment(number, bytes),
            Ok(Err(reason)) => Next::Failed(reason),
            Err(RecvTimeoutError::Timeout) => Next::Waiting,
            Err(RecvTimeoutError::Disconnected) => Next::Finished,
        }
    }
}

fn fetch(
    runtime: &Handle,
    downloader: &TrailerDownloader,
    segments: &Segments,
    first: u32,
    sender: &SyncSender<Result<(u32, Vec<u8>), String>>,
) {
    for number in first..=segments.last() {
        let url = segments.url(number);
        let result = download(runtime, || downloader.segment(&url));
        let failed = result.is_err();
        if sender.send(result.map(|bytes| (number, bytes))).is_err() || failed {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamepalate_app::trailer::Manifest;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = (8 + body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes
    }

    fn full_box(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
        let mut content = vec![version, 0, 0, 0];
        content.extend_from_slice(body);
        mp4_box(kind, &content)
    }

    fn segment(tfdt: Vec<u8>) -> Vec<u8> {
        let header = full_box(b"tfhd", 0, &1u32.to_be_bytes());
        let traf = mp4_box(b"traf", &[header, tfdt].concat());
        let moof = mp4_box(
            b"moof",
            &[full_box(b"mfhd", 0, &7u32.to_be_bytes()), traf].concat(),
        );
        [mp4_box(b"styp", b"msdh"), moof, mp4_box(b"mdat", &[0; 16])].concat()
    }

    fn init(elst: Vec<u8>) -> Vec<u8> {
        let header = full_box(b"tkhd", 0, &[0; 8]);
        let trak = mp4_box(b"trak", &[header, mp4_box(b"edts", &elst)].concat());
        let moov = mp4_box(b"moov", &[full_box(b"mvhd", 0, &[0; 8]), trak].concat());
        [mp4_box(b"ftyp", b"iso6"), moov].concat()
    }

    fn elst(version: u8, media_time: i64) -> Vec<u8> {
        let mut body = 1u32.to_be_bytes().to_vec();
        match version {
            1 => {
                body.extend_from_slice(&85_000u64.to_be_bytes());
                body.extend_from_slice(&media_time.to_be_bytes());
            }
            _ => {
                body.extend_from_slice(&85_000u32.to_be_bytes());
                body.extend_from_slice(&(media_time as i32).to_be_bytes());
            }
        }
        body.extend_from_slice(&[0, 1, 0, 0]);
        full_box(b"elst", version, &body)
    }

    #[test]
    fn reads_the_tfdt_of_a_segment() {
        let short = segment(full_box(b"tfdt", 0, &144_000u32.to_be_bytes()));
        let long = segment(full_box(b"tfdt", 1, &6_000_000_000u64.to_be_bytes()));

        assert_eq!(tfdt(&short), Some(144_000));
        assert_eq!(tfdt(&long), Some(6_000_000_000));
        assert_eq!(tfdt(&mp4_box(b"moof", &[])), None);
    }

    #[test]
    fn reads_the_aac_priming_from_the_edit_list() {
        assert_eq!(media_time(&init(elst(0, 1024))), 1024);
        assert_eq!(media_time(&init(elst(1, 1024))), 1024);
        assert_eq!(media_time(&init(mp4_box(b"free", &[]))), 0);
    }

    #[test]
    fn reads_the_timescale_of_the_track() {
        let version_0 = full_box(
            b"mdhd",
            0,
            &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0xbb, 0x80, 0, 0, 0, 0],
        );
        let mut long = vec![0; 16];
        long.extend_from_slice(&44_100u32.to_be_bytes());
        let version_1 = full_box(b"mdhd", 1, &long);
        let init_with = |mdhd: Vec<u8>| {
            let trak = mp4_box(b"trak", &mp4_box(b"mdia", &mdhd));
            mp4_box(b"moov", &trak)
        };

        assert_eq!(timescale(&init_with(version_0)), Some(48_000));
        assert_eq!(timescale(&init_with(version_1)), Some(44_100));
        assert_eq!(timescale(&init(elst(0, 0))), None);
    }

    #[test]
    fn a_broken_box_is_not_read_out_of_bounds() {
        let mut broken = segment(full_box(b"tfdt", 0, &1u32.to_be_bytes()));
        broken[3] = 200;

        assert_eq!(tfdt(&broken), None);
        assert_eq!(tfdt(&[0, 0, 0, 3, b'm', b'o', b'o', b'f']), None);
        let mut huge = mp4_box(b"styp", b"msdh");
        huge.extend_from_slice(&[0, 0, 0, 1]);
        huge.extend_from_slice(b"free");
        huge.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(tfdt(&huge), None);
    }

    #[test]
    fn the_time_of_a_sample_is_anchored_to_the_tfdt() {
        let timescale = 60_000;
        let tfdt = 180_180;

        assert_eq!(sample_time(tfdt, 180_180, 180_180, 0, timescale), 3.003);
        assert!((sample_time(tfdt, 0, 1001, 0, timescale) - 3.019_683).abs() < 1e-6);
        assert!((sample_time(48_000, 0, 0, 1024, 48_000) - 0.978_667).abs() < 1e-6);
    }

    fn hades() -> Segments {
        let xml = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT1M25.5S">
  <Period>
    <AdaptationSet contentType="video">
      <Representation id="0" codecs="av01.0.08M.08" bandwidth="2000000" width="1280" height="720">
        <SegmentTemplate timescale="1000000" duration="3000000" startNumber="1"
          initialization="init-stream$RepresentationID$.m4s"
          media="chunk-stream$RepresentationID$-$Number%05d$.m4s"/>
      </Representation>
    </AdaptationSet>
    <AdaptationSet contentType="audio">
      <Representation id="4" codecs="mp4a.40.2" bandwidth="128000">
        <SegmentTemplate timescale="1000000" duration="3000000" startNumber="1"
          initialization="init-stream$RepresentationID$.m4s"
          media="chunk-stream$RepresentationID$-$Number%05d$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let url = "https://video.akamai.steamstatic.com/store_trailers/1/2/dash_av1.mpd";

        Manifest::parse(url, xml).unwrap().video[0].segments.clone()
    }

    #[test]
    fn a_seek_finds_the_segment_from_the_real_duration() {
        let mut timeline = Timeline::new(&hades());
        assert_eq!(timeline.segment_at(0.0), 1);
        assert_eq!(timeline.segment_at(60.05), 21);

        timeline.learn(2, 3.003);

        assert_eq!(timeline.segment_at(60.05), 20);
        assert_eq!(timeline.segment_at(60.07), 21);
        assert_eq!(timeline.segment_at(-1.0), 1);
        assert_eq!(timeline.segment_at(500.0), 29);
    }

    #[test]
    fn the_audio_of_a_seek_starts_a_segment_before() {
        let timeline = Timeline::new(&hades());

        assert_eq!(timeline.segment_before(0.0), 1);
        assert_eq!(timeline.segment_before(4.0), 1);
        assert_eq!(timeline.segment_before(7.0), 2);
        assert_eq!(timeline.segment_before(500.0), 28);
    }

    #[test]
    fn the_first_segment_teaches_nothing() {
        let mut timeline = Timeline::new(&hades());
        timeline.learn(1, 0.0);
        timeline.learn(3, 0.0);

        assert_eq!(timeline.segment_at(3.001), 2);
    }
}
