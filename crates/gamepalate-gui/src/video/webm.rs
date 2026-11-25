use std::io::Cursor;
use std::time::Duration;

use gamepalate_app::trailer::TrailerDownloader;
use iced::widget::image;
use matroska_demuxer::{Frame as Block, MatroskaFile, TrackType};
use rusty_vp9::{DecodedFrame, Error as Vp9Error, Vp9Decoder};
use tokio::runtime::Handle;

use super::decode::i420_to_rgba;
use super::{Frame, Shared};

const VP9: &str = "V_VP9";
const WAIT: Duration = Duration::from_millis(50);
const NANOSECONDS: f64 = 1e9;
const MICROSECONDS: f64 = 1e6;
/// Times closer than this are the same packet, rounded to microseconds on the way through the decoder.
const SAME_TIME: f64 = 1e-5;
/// A lap of a still `webm`, so that the loop never spins.
const SHORTEST_LAP: f64 = 0.1;

/// A whole `webm`, read once: the VP9 packets in decoding order, with their times in seconds.
#[derive(Debug, PartialEq)]
pub struct Clip {
    pub packets: Vec<Packet>,
    pub duration: f64,
}

#[derive(Debug, PartialEq)]
pub struct Packet {
    pub pts: f64,
    pub data: Vec<u8>,
}

impl Clip {
    pub fn read(bytes: &[u8]) -> Result<Clip, String> {
        let mut file = MatroskaFile::open(Cursor::new(bytes))
            .map_err(|error| format!("could not read the video: {error}"))?;
        let scale = file.info().timestamp_scale().get() as f64 / NANOSECONDS;
        let track = file
            .tracks()
            .iter()
            .find(|track| track.track_type() == TrackType::Video && track.codec_id() == VP9)
            .ok_or_else(|| "the video has no VP9 track".to_string())?;
        let number = track.track_number().get();
        let frame_duration = track
            .default_duration()
            .map(|duration| duration.get() as f64 / NANOSECONDS);
        let declared = file.info().duration().map(|duration| duration * scale);

        let mut packets = Vec::new();
        let mut block = Block::default();
        while file
            .next_frame(&mut block)
            .map_err(|error| format!("could not read the video: {error}"))?
        {
            if block.track == number {
                packets.push(Packet {
                    pts: block.timestamp as f64 * scale,
                    data: std::mem::take(&mut block.data),
                });
            }
        }
        let duration = lap(&packets, frame_duration, declared)
            .ok_or_else(|| "the video has no frames".to_string())?;

        Ok(Clip { packets, duration })
    }
}

/// The last packet plus one frame; without a frame duration, the gap before the last packet.
fn lap(packets: &[Packet], frame_duration: Option<f64>, declared: Option<f64>) -> Option<f64> {
    let last = packets.last()?.pts;
    let gap = match packets {
        [.., before, last] => Some(last.pts - before.pts),
        _ => None,
    };
    let end = match frame_duration.or(gap) {
        Some(frame) => last + frame,
        None => declared.unwrap_or(last),
    };

    Some(end.max(SHORTEST_LAP))
}

pub fn play(shared: &Shared, runtime: Handle, url: String, rgba: bool) -> Result<(), String> {
    let Some(bytes) = download(shared, &runtime, url)? else {
        return Ok(());
    };
    let clip = Clip::read(&bytes)?;
    drop(bytes);
    let _ = shared.duration.set(Duration::from_secs_f64(clip.duration));

    run(shared, &clip, rgba)
}

/// The whole file, or nothing if the player stops first: the download does not hold the thread.
fn download(shared: &Shared, runtime: &Handle, url: String) -> Result<Option<Vec<u8>>, String> {
    let downloader = TrailerDownloader::new().map_err(|error| error.to_string())?;
    let task = runtime.spawn(async move { downloader.video(&url).await });
    while !task.is_finished() {
        if shared.is_over() {
            task.abort();
            return Ok(None);
        }
        std::thread::sleep(WAIT);
    }

    match runtime.block_on(task) {
        Ok(result) => result.map(Some).map_err(|error| error.to_string()),
        Err(error) => Err(format!("the video download stopped: {error}")),
    }
}

/// Plays the clip in laps until the player stops. The time keeps running from one lap to the
/// next, so the frames of the next lap queue up behind the last ones of this lap.
pub fn run(shared: &Shared, clip: &Clip, rgba: bool) -> Result<(), String> {
    let mut epoch = None;
    let mut lap = 0;
    let mut skip = 0.0;
    while !shared.is_over() {
        let current = shared.epoch();
        if epoch != Some(current) {
            epoch = Some(current);
            skip = shared.seek_target();
            lap = 0;
        }
        let offset = f64::from(lap) * clip.duration;
        decode_lap(shared, clip, current, offset, skip, rgba)?;
        lap += 1;
    }

    Ok(())
}

fn decode_lap(
    shared: &Shared,
    clip: &Clip,
    epoch: u64,
    offset: f64,
    skip: f64,
    rgba: bool,
) -> Result<(), String> {
    let mut decoder = Vp9Decoder::new();
    for packet in &clip.packets {
        let pts = ((offset + packet.pts) * MICROSECONDS).round() as i64;
        decoder
            .push(&packet.data, Some(pts))
            .map_err(|error| format!("the VP9 decoder failed: {error}"))?;
    }
    decoder.flush();

    while shared.is_current(epoch) {
        let decoded = match decoder.next_frame() {
            Ok(decoded) => decoded,
            Err(Vp9Error::Again) => continue,
            Err(Vp9Error::Eof) => break,
            Err(error) => return Err(format!("the VP9 decoder failed: {error}")),
        };
        let pts = decoded.pts.unwrap_or(0) as f64 / MICROSECONDS;
        if replaced_by(clip, offset, pts, skip) {
            continue;
        }
        if !shared.push_frame(frame(&decoded, epoch, pts, rgba)?) {
            break;
        }
    }

    Ok(())
}

/// Whether a later frame of the lap is already due at `skip`, so that this one would never show.
fn replaced_by(clip: &Clip, offset: f64, pts: f64, skip: f64) -> bool {
    clip.packets
        .iter()
        .map(|packet| offset + packet.pts)
        .any(|later| later > pts + SAME_TIME && later <= skip)
}

fn frame(decoded: &DecodedFrame, epoch: u64, pts: f64, rgba: bool) -> Result<Frame, String> {
    if decoded.bit_depth != 8 || decoded.subsampling_x != 1 || decoded.subsampling_y != 1 {
        return Err("the video is not 8-bit 4:2:0".to_string());
    }
    let (width, height) = (decoded.width as usize, decoded.height as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let y = tight(decoded, 0, width, height)?;
    let u = tight(decoded, 1, chroma_width, chroma_height)?;
    let v = tight(decoded, 2, chroma_width, chroma_height)?;
    let rgba = rgba.then(|| {
        image::Handle::from_rgba(
            width as u32,
            height as u32,
            i420_to_rgba(width, height, &y, &u, &v),
        )
    });

    Ok(Frame {
        epoch,
        pts,
        width: width as u32,
        height: height as u32,
        y,
        u,
        v,
        rgba,
    })
}

fn tight(
    decoded: &DecodedFrame,
    plane: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, String> {
    let short = || "a VP9 frame is cut short".to_string();
    let bytes = decoded.planes.get(plane).ok_or_else(short)?;
    let stride = *decoded.strides.get(plane).ok_or_else(short)?;
    let mut tight = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = row * stride;
        tight.extend_from_slice(bytes.get(start..start + width).ok_or_else(short)?);
    }

    Ok(tight)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use rusty_vp9::Vp9Encoder;

    use super::super::{Options, Source, State, lock};
    use super::*;

    fn element(id: u32, body: &[u8]) -> Vec<u8> {
        let id = id.to_be_bytes();
        let start = id.iter().position(|byte| *byte != 0).unwrap_or(3);
        let mut bytes = id[start..].to_vec();
        bytes.push(0x01);
        bytes.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        bytes.extend_from_slice(body);
        bytes
    }

    fn unsigned(id: u32, value: u64) -> Vec<u8> {
        element(id, &value.to_be_bytes())
    }

    struct Track {
        number: u8,
        kind: u64,
        codec: &'static str,
        default_duration: Option<u64>,
    }

    const VIDEO: Track = Track {
        number: 1,
        kind: 1,
        codec: VP9,
        default_duration: Some(100_000_000),
    };

    /// A `webm` with one cluster; the blocks are (track, milliseconds, data), with the default
    /// timestamp scale of a millisecond.
    fn webm(tracks: &[Track], blocks: &[(u8, i16, &[u8])]) -> Vec<u8> {
        let header = [
            element(0x4282, b"webm"),
            unsigned(0x4287, 4),
            unsigned(0x4285, 2),
        ]
        .concat();
        let info = [
            unsigned(0x2AD7B1, 1_000_000),
            element(0x4D80, b"test"),
            element(0x5741, b"test"),
        ]
        .concat();
        let entries: Vec<u8> = tracks
            .iter()
            .flat_map(|track| {
                let mut body = [
                    unsigned(0xD7, u64::from(track.number)),
                    unsigned(0x73C5, u64::from(track.number)),
                    unsigned(0x83, track.kind),
                    element(0x86, track.codec.as_bytes()),
                ]
                .concat();
                if let Some(duration) = track.default_duration {
                    body.extend(unsigned(0x23E383, duration));
                }
                if track.kind == 1 {
                    let size = [unsigned(0xB0, 16), unsigned(0xBA, 16)].concat();
                    body.extend(element(0xE0, &size));
                }
                element(0xAE, &body)
            })
            .collect();
        let mut cluster = unsigned(0xE7, 0);
        for (track, time, data) in blocks {
            let mut body = vec![0x80 | track];
            body.extend_from_slice(&time.to_be_bytes());
            body.push(0x80);
            body.extend_from_slice(data);
            cluster.extend(element(0xA3, &body));
        }
        let segment = [
            element(0x1549A966, &info),
            element(0x1654AE6B, &entries),
            element(0x1F43B675, &cluster),
        ]
        .concat();

        [element(0x1A45DFA3, &header), element(0x18538067, &segment)].concat()
    }

    /// Three 16 × 16 VP9 frames, each a lighter grey.
    fn vp9_frames() -> Vec<Vec<u8>> {
        let mut encoder = Vp9Encoder::default();
        for grey in [40, 120, 200] {
            let y = vec![grey; 16 * 16];
            let chroma = vec![128; 8 * 8];
            encoder
                .push_frame([&y, &chroma, &chroma], [16, 8, 8], 16, 16)
                .unwrap();
        }
        encoder.flush();
        let mut packets = Vec::new();
        while let Ok(packet) = encoder.next_packet() {
            packets.push(packet.data);
        }
        packets
    }

    fn clip() -> Clip {
        let frames = vp9_frames();
        let bytes = webm(
            &[VIDEO],
            &[
                (1, 0, &frames[0]),
                (1, 100, &frames[1]),
                (1, 200, &frames[2]),
            ],
        );
        Clip::read(&bytes).unwrap()
    }

    fn looping() -> Arc<Shared> {
        let source = Source::Loop {
            url: "https://a/extras/b.webm".to_string(),
        };
        Arc::new(Shared::for_source(&source, Options::default()))
    }

    fn play_in_a_thread(
        clip: Clip,
        rgba: bool,
    ) -> (Arc<Shared>, std::thread::JoinHandle<Result<(), String>>) {
        let shared = looping();
        let player = Arc::clone(&shared);
        let thread = std::thread::spawn(move || run(&player, &clip, rgba));
        (shared, thread)
    }

    /// Takes the next `count` frames off the queue, as (epoch, pts in ms, first luma byte).
    fn take(shared: &Shared, count: usize) -> Vec<(u64, i64, u8)> {
        let mut taken = Vec::new();
        let start = Instant::now();
        while taken.len() < count && start.elapsed() < Duration::from_secs(10) {
            let frame = lock(&shared.frames).queue.pop_front();
            shared.frames_changed.notify_all();
            match frame {
                Some(frame) => {
                    taken.push((frame.epoch, (frame.pts * 1000.0).round() as i64, frame.y[0]))
                }
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        taken
    }

    #[test]
    fn reads_the_times_of_the_vp9_blocks() {
        let audio = Track {
            number: 2,
            kind: 2,
            codec: "A_OPUS",
            default_duration: None,
        };
        let bytes = webm(
            &[VIDEO, audio],
            &[
                (1, 0, b"a"),
                (2, 0, b"sound"),
                (1, 100, b"b"),
                (1, 250, b"c"),
            ],
        );

        let clip = Clip::read(&bytes).unwrap();

        let packet = |pts: f64, data: &[u8]| Packet {
            pts,
            data: data.to_vec(),
        };
        assert_eq!(
            clip.packets,
            vec![packet(0.0, b"a"), packet(0.1, b"b"), packet(0.25, b"c")]
        );
        assert!((clip.duration - 0.35).abs() < 1e-9);
    }

    #[test]
    fn without_a_frame_duration_the_lap_ends_one_gap_after_the_last_block() {
        let track = Track {
            default_duration: None,
            ..VIDEO
        };
        let bytes = webm(&[track], &[(1, 0, b"a"), (1, 50, b"b")]);

        assert!((Clip::read(&bytes).unwrap().duration - 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_still_video_still_takes_a_moment() {
        let track = Track {
            default_duration: None,
            ..VIDEO
        };
        let bytes = webm(&[track], &[(1, 0, b"a")]);

        assert_eq!(Clip::read(&bytes).unwrap().duration, SHORTEST_LAP);
    }

    #[test]
    fn refuses_a_webm_without_vp9_frames() {
        let vp8 = Track {
            codec: "V_VP8",
            ..VIDEO
        };

        assert_eq!(
            Clip::read(&webm(&[vp8], &[(1, 0, b"a")])),
            Err("the video has no VP9 track".to_string())
        );
        assert_eq!(
            Clip::read(&webm(&[VIDEO], &[])),
            Err("the video has no frames".to_string())
        );
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let bytes = webm(&[VIDEO], &[(1, 0, b"a"), (1, 100, b"b")]);

        assert!(Clip::read(b"<html>").is_err());
        assert!(Clip::read(&bytes[..bytes.len() / 2]).is_err());
    }

    #[test]
    fn plays_in_laps_with_the_time_running_on() {
        let (shared, thread) = play_in_a_thread(clip(), false);

        let frames = take(&shared, 7);
        shared.stop();

        assert_eq!(thread.join().unwrap(), Ok(()));
        let times: Vec<_> = frames
            .iter()
            .map(|(epoch, pts, _)| (*epoch, *pts))
            .collect();
        assert_eq!(
            times,
            vec![
                (0, 0),
                (0, 100),
                (0, 200),
                (0, 300),
                (0, 400),
                (0, 500),
                (0, 600)
            ]
        );
        let lumas: Vec<_> = frames.iter().map(|(_, _, luma)| *luma).collect();
        assert_eq!(lumas[..3], lumas[3..6]);
        assert!(lumas[0] < lumas[1] && lumas[1] < lumas[2]);
    }

    #[test]
    fn a_loop_never_ends() {
        let (shared, thread) = play_in_a_thread(clip(), false);
        let first = take(&shared, 1);
        shared.drawn(first[0].0);

        take(&shared, 10);

        assert_eq!(shared.state(), State::Playing);
        shared.stop();
        assert_eq!(thread.join().unwrap(), Ok(()));
    }

    #[test]
    fn the_queue_holds_eight_frames() {
        let (shared, thread) = play_in_a_thread(clip(), true);

        let start = Instant::now();
        while lock(&shared.frames).queue.len() < 8 && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(50));
        let (queued, rgba) = {
            let frames = lock(&shared.frames);
            (frames.queue.len(), frames.queue[0].rgba.is_some())
        };
        shared.stop();

        assert_eq!(thread.join().unwrap(), Ok(()));
        assert_eq!(queued, 8);
        assert!(rgba);
    }

    #[test]
    fn a_seek_starts_from_the_frame_on_screen_at_that_time() {
        let (shared, thread) = play_in_a_thread(clip(), false);
        take(&shared, 2);

        shared.seek(0.15);
        let after: Vec<_> = take(&shared, 3)
            .into_iter()
            .filter(|(epoch, _, _)| *epoch == 1)
            .map(|(_, pts, _)| pts)
            .collect();
        shared.stop();

        assert_eq!(thread.join().unwrap(), Ok(()));
        assert_eq!(after[..2], [100, 200]);
    }

    #[test]
    fn a_broken_packet_stops_the_video_with_an_error() {
        let bytes = webm(&[VIDEO], &[(1, 0, &[0; 16]), (1, 100, &[0; 16])]);
        let clip = Clip::read(&bytes).unwrap();

        let result = run(&looping(), &clip, false);

        assert!(matches!(result, Err(reason) if reason.starts_with("the VP9 decoder failed")));
    }
}
