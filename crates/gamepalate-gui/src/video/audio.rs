use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::Shared;

#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pts: f64,
    at: Instant,
    running: bool,
    ahead: f64,
}

impl Clock {
    pub fn stopped(pts: f64) -> Clock {
        Clock {
            pts,
            at: Instant::now(),
            running: false,
            ahead: 0.0,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn at(&self, now: Instant) -> f64 {
        if !self.running {
            return self.pts;
        }
        let elapsed = match now.checked_duration_since(self.at) {
            Some(elapsed) => elapsed.as_secs_f64(),
            None => -(self.at - now).as_secs_f64(),
        };

        self.pts + elapsed.min(self.ahead)
    }

    fn heard(&mut self, pts: f64, at: Instant, ahead: f64) {
        *self = Clock {
            pts,
            at,
            running: true,
            ahead,
        };
    }

    fn halt(&mut self, now: Instant) {
        if self.running {
            self.pts = self.at(now);
            self.running = false;
        }
    }
}

pub struct AudioBuffer {
    pub epoch: u64,
    pub rate: f64,
    pub complete: bool,
    head: f64,
    samples: VecDeque<f32>,
}

impl AudioBuffer {
    pub fn new(epoch: u64, at: f64) -> AudioBuffer {
        AudioBuffer {
            epoch,
            rate: 48_000.0,
            complete: false,
            head: at,
            samples: VecDeque::new(),
        }
    }

    pub fn restart(&mut self, epoch: u64, at: f64) {
        self.epoch = epoch;
        self.complete = false;
        self.head = at;
        self.samples.clear();
    }

    pub fn seconds(&self) -> f64 {
        (self.samples.len() / 2) as f64 / self.rate
    }

    pub fn push(&mut self, epoch: u64, pts: f64, stereo: &[f32]) -> bool {
        if epoch != self.epoch {
            return false;
        }
        if self.samples.is_empty() {
            self.head = pts;
        }
        self.samples.extend(stereo);

        true
    }

    fn is_drained(&self) -> bool {
        self.complete && self.samples.len() < 4
    }
}

pub struct Resampler {
    step: f64,
    position: f64,
}

impl Resampler {
    pub fn new(from: f64, to: f64) -> Resampler {
        Resampler {
            step: from / to,
            position: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.position = 0.0;
    }

    pub fn pts(&self, buffer: &AudioBuffer) -> f64 {
        buffer.head + self.position / buffer.rate
    }

    pub fn fill(&mut self, buffer: &mut AudioBuffer, out: &mut [f32]) -> usize {
        let available = buffer.samples.len() / 2;
        let mut produced = 0;
        for frame in out.as_chunks_mut::<2>().0 {
            let index = self.position as usize;
            let weight = (self.position - index as f64) as f32;
            let needed = if weight > 0.0 { index + 1 } else { index };
            if needed >= available {
                break;
            }
            let (left, right) = (buffer.samples[2 * index], buffer.samples[2 * index + 1]);
            if weight > 0.0 {
                let (next_left, next_right) =
                    (buffer.samples[2 * index + 2], buffer.samples[2 * index + 3]);
                frame[0] = left + (next_left - left) * weight;
                frame[1] = right + (next_right - right) * weight;
            } else {
                frame[0] = left;
                frame[1] = right;
            }
            self.position += self.step;
            produced += 1;
        }
        let consumed = (self.position as usize).min(available);
        buffer.samples.drain(..consumed * 2);
        buffer.head += consumed as f64 / buffer.rate;
        self.position -= consumed as f64;

        produced
    }
}

pub fn open(shared: &Arc<Shared>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "there is no audio output device".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|error| format!("the audio device has no usable output: {error}"))?;
    tracing::info!("trailer audio: {:?}, {config:?}", host.id());
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let stream = match format {
        cpal::SampleFormat::I16 => build::<i16>(&device, config, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, shared),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, shared),
        cpal::SampleFormat::F64 => build::<f64>(&device, config, shared),
        _ => build::<f32>(&device, config, shared),
    }
    .map_err(|error| format!("could not open the audio output: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("could not start the audio output: {error}"))?;

    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let rate = f64::from(config.sample_rate);
    let mut resampler = Resampler::new(48_000.0, rate);
    let mut source_rate = 48_000.0;
    let mut heard_epoch = None;
    let mut scratch = Vec::new();
    let output = Arc::clone(shared);
    let failure = Arc::clone(shared);

    device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let now = Instant::now();
            let timestamp = info.timestamp();
            let latency = timestamp.playback.duration_since(timestamp.callback);
            let frames = data.len() / channels;
            scratch.clear();
            scratch.resize(frames * 2, 0.0);

            let epoch = output.epoch();
            let audible = output.is_audible(epoch);
            let mut produced = 0;
            {
                let mut buffer = output.audio();
                if buffer.rate != source_rate {
                    source_rate = buffer.rate;
                    resampler = Resampler::new(source_rate, rate);
                }
                if heard_epoch != Some(buffer.epoch) {
                    heard_epoch = Some(buffer.epoch);
                    resampler.reset();
                }
                let pts = resampler.pts(&buffer);
                if audible && buffer.epoch == epoch {
                    produced = resampler.fill(&mut buffer, &mut scratch);
                    if produced < frames && buffer.is_drained() {
                        output.audio_played_out(epoch);
                    }
                }
                let mut clock = output.clock();
                if produced > 0 {
                    clock.heard(pts, now + latency, produced as f64 / rate);
                } else {
                    clock.halt(now);
                }
            }

            let gain = output.gain();
            for (frame, stereo) in data.chunks_mut(channels).zip(scratch.as_chunks::<2>().0) {
                let [left, right] = stereo.map(|sample| sample * gain);
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let value = match (channels, channel) {
                        (1, _) => (left + right) * 0.5,
                        (_, 0) => left,
                        (_, 1) => right,
                        _ => 0.0,
                    };
                    *sample = T::from_sample(value);
                }
            }
        },
        move |error: cpal::Error| {
            if stops_the_output(error.kind()) {
                failure.fail(format!("the audio output stopped: {error}"));
            } else {
                tracing::debug!("trailer audio output: {error}");
            }
        },
        None,
    )
}

fn stops_the_output(kind: cpal::ErrorKind) -> bool {
    matches!(
        kind,
        cpal::ErrorKind::DeviceNotAvailable
            | cpal::ErrorKind::HostUnavailable
            | cpal::ErrorKind::PermissionDenied
            | cpal::ErrorKind::StreamInvalidated
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect()
    }

    #[test]
    fn an_underrun_does_not_stop_the_trailer() {
        assert!(!stops_the_output(cpal::ErrorKind::Xrun));
        assert!(!stops_the_output(cpal::ErrorKind::DeviceChanged));
        assert!(!stops_the_output(cpal::ErrorKind::BackendError));
        assert!(stops_the_output(cpal::ErrorKind::DeviceNotAvailable));
        assert!(stops_the_output(cpal::ErrorKind::StreamInvalidated));
    }

    #[test]
    fn the_clock_runs_from_the_heard_sample() {
        let start = Instant::now();
        let mut clock = Clock::stopped(10.0);
        assert_eq!(clock.at(start + Duration::from_secs(1)), 10.0);

        clock.heard(10.0, start + Duration::from_millis(50), 0.02);

        assert!((clock.at(start + Duration::from_millis(60)) - 10.01).abs() < 1e-9);
        assert!((clock.at(start + Duration::from_millis(40)) - 9.99).abs() < 1e-9);
        assert!((clock.at(start + Duration::from_secs(1)) - 10.02).abs() < 1e-9);
    }

    #[test]
    fn a_halted_clock_keeps_its_time() {
        let start = Instant::now();
        let mut clock = Clock::stopped(0.0);
        clock.heard(5.0, start, 1.0);

        clock.halt(start + Duration::from_millis(250));

        assert!(!clock.is_running());
        assert!((clock.at(start + Duration::from_secs(9)) - 5.25).abs() < 1e-9);
    }

    #[test]
    fn resamples_48_to_96_khz() {
        let mut buffer = AudioBuffer::new(0, 2.0);
        buffer.push(0, 2.0, &ramp(100));
        let mut resampler = Resampler::new(48_000.0, 96_000.0);
        let mut out = vec![0.0; 20];

        let produced = resampler.fill(&mut buffer, &mut out);

        assert_eq!(produced, 10);
        assert_eq!(&out[..8], &[0.0, 0.0, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5]);
        assert_eq!(buffer.samples.len(), 95 * 2);
        assert!((resampler.pts(&buffer) - (2.0 + 5.0 / 48_000.0)).abs() < 1e-12);
    }

    #[test]
    fn resamples_48_to_44_1_khz() {
        let mut buffer = AudioBuffer::new(0, 0.0);
        buffer.push(0, 0.0, &ramp(4800));
        let mut resampler = Resampler::new(48_000.0, 44_100.0);
        let mut out = vec![0.0; 441 * 2];

        let produced = resampler.fill(&mut buffer, &mut out);

        assert_eq!(produced, 441);
        let step = 48_000.0 / 44_100.0;
        assert!((out[2 * 100] - 100.0 * step as f32).abs() < 1e-3);
        assert!((out[2 * 100 + 1] + 100.0 * step as f32).abs() < 1e-3);
        assert!((resampler.pts(&buffer) - 0.01).abs() < 1e-9);
    }

    #[test]
    fn the_resampler_stops_at_the_end_of_the_buffer() {
        let mut buffer = AudioBuffer::new(0, 0.0);
        buffer.push(0, 0.0, &ramp(3));
        let mut resampler = Resampler::new(48_000.0, 48_000.0);
        let mut out = vec![9.0; 10];

        assert_eq!(resampler.fill(&mut buffer, &mut out), 3);
        assert_eq!(&out[..6], &[0.0, 0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(out[6], 9.0);
    }

    #[test]
    fn samples_of_an_old_epoch_are_thrown_away() {
        let mut buffer = AudioBuffer::new(0, 0.0);
        buffer.push(0, 0.0, &ramp(10));

        buffer.restart(1, 30.0);

        assert!(!buffer.push(0, 0.5, &ramp(10)));
        assert_eq!(buffer.seconds(), 0.0);
        assert!(buffer.push(1, 30.01, &ramp(48)));
        assert_eq!(buffer.head, 30.01);
        assert!((buffer.seconds() - 0.001).abs() < 1e-12);
    }
}
