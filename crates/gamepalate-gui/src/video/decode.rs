use std::sync::Arc;
use std::time::Duration;

use dav1d::{Decoder, Error as Av1Error, PixelLayout, PlanarImageComponent, Settings};
use gamepalate_app::trailer::{AudioRendition, TrailerDownloader, VideoRendition};
use iced::widget::image;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use tokio::runtime::Handle;

use super::stream::{self, Fetcher, Next, Timeline};
use super::{Frame, Shared, Source, audio, webm};

const WAIT: Duration = Duration::from_millis(50);
const AUDIO_AHEAD: f64 = 10.0;
const DEFAULT_RATE: u32 = 48_000;

pub fn start(shared: Arc<Shared>, runtime: Handle, source: Source, rgba: bool) {
    let thread = Arc::clone(&shared);
    let name = match source {
        Source::Dash { .. } => "trailer-video",
        Source::Loop { .. } => "description-video",
    };
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let played = match source {
                Source::Dash {
                    manifest_url,
                    height,
                } => play(&thread, runtime, &manifest_url, height, rgba),
                Source::Loop { url } => webm::play(&thread, runtime, url, rgba),
            };
            if let Err(reason) = played {
                thread.fail(reason);
            }
        });
    if let Err(error) = spawned {
        shared.fail(format!("could not start the trailer: {error}"));
    }
}

fn play(
    shared: &Arc<Shared>,
    runtime: Handle,
    manifest_url: &str,
    height: u32,
    rgba: bool,
) -> Result<(), String> {
    let downloader = Arc::new(TrailerDownloader::new().map_err(|error| error.to_string())?);
    let manifest = runtime
        .block_on(downloader.manifest(manifest_url))
        .map_err(|error| error.to_string())?;
    let _ = shared.duration.set(manifest.duration);
    let video = manifest.video_for_height(height).clone();
    let audio = manifest.audio.clone();

    let audio_shared = Arc::clone(shared);
    let audio_runtime = runtime.clone();
    let audio_downloader = Arc::clone(&downloader);
    std::thread::Builder::new()
        .name("trailer-audio".to_string())
        .spawn(move || {
            if let Err(reason) = play_audio(&audio_shared, audio_runtime, audio_downloader, audio) {
                audio_shared.fail(reason);
            }
        })
        .map_err(|error| format!("could not start the trailer audio: {error}"))?;

    play_video(shared, runtime, downloader, video, rgba)
}

fn play_video(
    shared: &Shared,
    runtime: Handle,
    downloader: Arc<TrailerDownloader>,
    rendition: VideoRendition,
    rgba: bool,
) -> Result<(), String> {
    let init = stream::download(&runtime, || {
        downloader.segment(rendition.segments.init_url())
    })?;
    let media_time = stream::media_time(&init);
    let mut settings = Settings::new();
    settings.set_n_threads(0);
    let mut decoder = Decoder::with_settings(&settings)
        .map_err(|error| format!("could not start the AV1 decoder: {error}"))?;
    let mut timeline = Timeline::new(&rendition.segments);
    let mut epoch = None;
    let mut fetcher = None;
    let mut skip = 0.0;

    while !shared.is_over() {
        let current = shared.epoch();
        if epoch != Some(current) {
            epoch = Some(current);
            skip = shared.seek_target();
            decoder.flush();
            fetcher = Some(Fetcher::start(
                runtime.clone(),
                Arc::clone(&downloader),
                rendition.segments.clone(),
                timeline.segment_at(skip),
            )?);
        }
        let Some(segments) = fetcher.as_ref() else {
            std::thread::sleep(WAIT);
            continue;
        };
        let mut sink = VideoSink {
            shared,
            epoch: current,
            skip,
            frame_duration: 0.0,
            rgba,
        };
        match segments.next(WAIT) {
            Next::Segment(number, segment) => {
                let tfdt = stream::tfdt(&segment)
                    .ok_or_else(|| format!("video segment {number} has no tfdt"))?;
                let mut file = init.clone();
                file.extend_from_slice(&segment);
                let movie = re_mp4::Mp4::read_bytes(&file)
                    .map_err(|error| format!("could not read video segment {number}: {error}"))?;
                let track = movie
                    .tracks()
                    .values()
                    .find(|track| track.kind == Some(re_mp4::TrackKind::Video))
                    .ok_or_else(|| format!("video segment {number} has no video track"))?;
                let timescale = track.timescale.max(1);
                timeline.learn(number, (tfdt - media_time) as f64 / timescale as f64);
                let first = track
                    .samples
                    .first()
                    .map(|sample| sample.composition_timestamp)
                    .unwrap_or(0);
                for sample in &track.samples {
                    if !shared.is_current(current) {
                        break;
                    }
                    let pts = stream::sample_time(
                        tfdt,
                        first,
                        sample.composition_timestamp,
                        media_time,
                        timescale,
                    );
                    sink.frame_duration = sample.duration as f64 / timescale as f64;
                    let data = file
                        .get(sample.byte_range())
                        .ok_or_else(|| format!("video segment {number} is cut short"))?
                        .to_vec();
                    send(&mut decoder, &mut sink, data, pts)?;
                }
            }
            Next::Waiting => {}
            Next::Finished => {
                while sink.drain_one(&mut decoder)? {}
                if shared.is_current(current) {
                    shared.video_finished(current);
                }
                fetcher = None;
            }
            Next::Failed(reason) => return Err(reason),
        }
    }

    Ok(())
}

fn send(
    decoder: &mut Decoder,
    sink: &mut VideoSink,
    data: Vec<u8>,
    pts: f64,
) -> Result<(), String> {
    let timestamp = (pts * 1e6).round() as i64;
    match decoder.send_data(data, None, Some(timestamp), None) {
        Ok(()) => {}
        Err(Av1Error::Again) => loop {
            sink.drain_one(decoder)?;
            match decoder.send_pending_data() {
                Ok(()) => break,
                Err(Av1Error::Again) => {}
                Err(error) => return Err(format!("the AV1 decoder failed: {error}")),
            }
        },
        Err(error) => return Err(format!("the AV1 decoder failed: {error}")),
    }
    sink.drain_one(decoder)?;

    Ok(())
}

struct VideoSink<'a> {
    shared: &'a Shared,
    epoch: u64,
    skip: f64,
    frame_duration: f64,
    rgba: bool,
}

impl VideoSink<'_> {
    fn drain_one(&mut self, decoder: &mut Decoder) -> Result<bool, String> {
        let picture = match decoder.get_picture() {
            Ok(picture) => picture,
            Err(Av1Error::Again) => return Ok(false),
            Err(error) => return Err(format!("the AV1 decoder failed: {error}")),
        };
        let pts = picture.timestamp().unwrap_or(0) as f64 / 1e6;
        if pts + self.frame_duration <= self.skip {
            return Ok(true);
        }
        if picture.pixel_layout() != PixelLayout::I420 || picture.bit_depth() != 8 {
            return Err("the trailer is not 8-bit 4:2:0 video".to_string());
        }
        let (width, height) = (picture.width() as usize, picture.height() as usize);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let copy = |component: PlanarImageComponent, width: usize, height: usize| {
            let stride = picture.stride(component) as usize;
            let plane = picture.plane(component);
            let bytes: &[u8] = plane.as_ref();
            let mut tight = Vec::with_capacity(width * height);
            for row in 0..height {
                tight.extend_from_slice(&bytes[row * stride..row * stride + width]);
            }
            tight
        };
        let y = copy(PlanarImageComponent::Y, width, height);
        let u = copy(PlanarImageComponent::U, chroma_width, chroma_height);
        let v = copy(PlanarImageComponent::V, chroma_width, chroma_height);
        let rgba = self.rgba.then(|| {
            image::Handle::from_rgba(
                width as u32,
                height as u32,
                i420_to_rgba(width, height, &y, &u, &v),
            )
        });
        let frame = Frame {
            epoch: self.epoch,
            pts,
            width: width as u32,
            height: height as u32,
            y,
            u,
            v,
            rgba,
        };

        Ok(self.shared.push_frame(frame))
    }
}

pub fn i420_to_rgba(width: usize, height: usize, y: &[u8], u: &[u8], v: &[u8]) -> Vec<u8> {
    let chroma_width = width.div_ceil(2);
    let mut rgba = vec![255; width * height * 4];
    for row in 0..height {
        for column in 0..width {
            let luma = (i32::from(y[row * width + column]) - 16) * 298;
            let chroma = (row / 2) * chroma_width + column / 2;
            let blue = i32::from(u[chroma]) - 128;
            let red = i32::from(v[chroma]) - 128;
            let pixel = (row * width + column) * 4;
            rgba[pixel] = ((luma + 459 * red + 128) >> 8).clamp(0, 255) as u8;
            rgba[pixel + 1] = ((luma - 55 * blue - 136 * red + 128) >> 8).clamp(0, 255) as u8;
            rgba[pixel + 2] = ((luma + 541 * blue + 128) >> 8).clamp(0, 255) as u8;
        }
    }

    rgba
}

fn play_audio(
    shared: &Arc<Shared>,
    runtime: Handle,
    downloader: Arc<TrailerDownloader>,
    rendition: AudioRendition,
) -> Result<(), String> {
    let _output = audio::open(shared)?;
    let init = stream::download(&runtime, || {
        downloader.segment(rendition.segments.init_url())
    })?;
    let priming = stream::media_time(&init);
    let timescale = stream::timescale(&init).unwrap_or(u64::from(DEFAULT_RATE));
    let timeline = Timeline::new(&rendition.segments);
    let mut decoder = AacDecoder::default();
    let mut epoch = None;
    let mut fetcher = None;
    let mut skip = 0.0;

    while !shared.is_over() {
        let current = shared.epoch();
        if epoch != Some(current) {
            epoch = Some(current);
            skip = shared.seek_target();
            decoder.reset();
            fetcher = Some(Fetcher::start(
                runtime.clone(),
                Arc::clone(&downloader),
                rendition.segments.clone(),
                timeline.segment_before(skip),
            )?);
        }
        if shared.audio().seconds() > AUDIO_AHEAD {
            std::thread::sleep(WAIT);
            continue;
        }
        let Some(segments) = fetcher.as_ref() else {
            std::thread::sleep(WAIT);
            continue;
        };
        match segments.next(WAIT) {
            Next::Segment(number, segment) => {
                let tfdt = stream::tfdt(&segment)
                    .ok_or_else(|| format!("audio segment {number} has no tfdt"))?;
                let start = (tfdt - priming) as f64 / timescale as f64;
                let mut file = init.clone();
                file.extend_from_slice(&segment);
                decoder.decode(shared, current, file, start, skip)?;
            }
            Next::Waiting => {}
            Next::Finished => {
                let mut buffer = shared.audio();
                if buffer.epoch == current {
                    buffer.complete = true;
                }
                fetcher = None;
            }
            Next::Failed(reason) => return Err(reason),
        }
    }

    Ok(())
}

#[derive(Default)]
struct AacDecoder {
    decoder: Option<Box<dyn AudioDecoder>>,
    interleaved: Vec<f32>,
    stereo: Vec<f32>,
}

impl AacDecoder {
    fn reset(&mut self) {
        if let Some(decoder) = self.decoder.as_mut() {
            decoder.reset();
        }
    }

    fn decode(
        &mut self,
        shared: &Shared,
        epoch: u64,
        file: Vec<u8>,
        start: f64,
        skip: f64,
    ) -> Result<(), String> {
        let source =
            MediaSourceStream::new(Box::new(std::io::Cursor::new(file)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("mp4");
        let mut format = symphonia::default::get_probe()
            .probe(
                &hint,
                source,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|error| format!("could not read an audio segment: {error}"))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| "an audio segment has no audio track".to_string())?;
        let id = track.id;
        let parameters = track
            .codec_params
            .as_ref()
            .and_then(|parameters| parameters.audio())
            .ok_or_else(|| "an audio segment has no audio parameters".to_string())?
            .clone();
        let rate = f64::from(parameters.sample_rate.unwrap_or(DEFAULT_RATE));
        if self.decoder.is_none() {
            let decoder = symphonia::default::get_codecs()
                .make_audio_decoder(&parameters, &AudioDecoderOptions::default())
                .map_err(|error| format!("could not start the AAC decoder: {error}"))?;
            shared.audio().rate = rate;
            self.decoder = Some(decoder);
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(());
        };

        let mut position = start;
        while let Ok(Some(packet)) = format.next_packet() {
            if packet.track_id != id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(error) => {
                    tracing::debug!("trailer audio packet: {error}");
                    continue;
                }
            };
            let channels = decoded.spec().channels().count().max(1);
            self.interleaved.clear();
            decoded.copy_to_vec_interleaved::<f32>(&mut self.interleaved);
            self.stereo.clear();
            for frame in self.interleaved.chunks_exact(channels) {
                let left = frame[0];
                let right = frame.get(1).copied().unwrap_or(left);
                self.stereo.extend_from_slice(&[left, right]);
            }
            let frames = self.stereo.len() / 2;
            let pts = position;
            position += frames as f64 / rate;
            let skipped = (((skip - pts) * rate).ceil().max(0.0) as usize).min(frames);
            if skipped == frames {
                continue;
            }
            let pts = pts + skipped as f64 / rate;
            if !shared.audio().push(epoch, pts, &self.stereo[skipped * 2..]) {
                break;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bt709_limited_range_to_rgba() {
        let rgba = i420_to_rgba(2, 2, &[16, 235, 16, 235], &[128], &[128]);

        assert_eq!(&rgba[..4], &[0, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[255, 255, 255, 255]);
        assert_eq!(rgba.len(), 16);
    }
}
