//! Meeting audio capture: the microphone and the system output, each streamed
//! to its own 16 kHz mono WAV on disk, so a three-hour meeting never sits in
//! memory.
//!
//! Both tracks share one timeline that starts when the meeting starts. When a
//! source falls silent (a device was unplugged, the output device changed),
//! its writer fills the gap with silence, so a sample index means the same
//! moment in both files.

pub mod mic;
pub mod system;

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::coordinator::{CaptureBackend, Track};
use super::paths::{MIC_TRACK, SYSTEM_TRACK};
use crate::audio_toolkit::audio::FrameResampler;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;

const RATE: u64 = TARGET_SAMPLE_RATE as u64;
/// A source this far behind the wall clock has stopped delivering audio.
/// Ordinary buffering stays well below it.
const GAP_TOLERANCE: u64 = RATE / 2;
/// How often the header is rewritten, which bounds what a crash can lose
/// track of (`wav_recovery` restores the rest from the file length).
const HEADER_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Audio callbacks deliver every few milliseconds; this holds several seconds.
const QUEUE_CHUNKS: usize = 1024;

/// Samples of silence to write so a track that has `written` samples catches
/// up with the `expected` count the wall clock implies.
pub fn silence_to_insert(written: u64, expected: u64) -> u64 {
    if expected > written + GAP_TOLERANCE {
        expected - written
    } else {
        0
    }
}

fn samples_in(duration: Duration) -> u64 {
    (duration.as_secs_f64() * RATE as f64).round() as u64
}

enum Message {
    Samples { data: Vec<f32>, sample_rate: u32 },
    Finish { total: u64 },
}

/// Hands mono audio from a capture callback to its writer. It never blocks:
/// a full queue drops the chunk, and the gap is filled with silence.
#[derive(Clone)]
pub struct Feeder(SyncSender<Message>);

impl Feeder {
    pub fn push(&self, data: Vec<f32>, sample_rate: u32) {
        if sample_rate > 0 && !data.is_empty() {
            let _ = self.0.try_send(Message::Samples { data, sample_rate });
        }
    }
}

pub struct TrackWriter {
    sender: SyncSender<Message>,
    failed: Arc<AtomicBool>,
    thread: JoinHandle<Result<u64, String>>,
}

impl TrackWriter {
    /// Creates the WAV at `path`; `origin` is when the meeting started.
    pub fn create(path: &Path, origin: Instant) -> Result<Self, String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav = hound::WavWriter::create(path, spec)
            .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CHUNKS);
        let failed = Arc::new(AtomicBool::new(false));
        let thread_failed = Arc::clone(&failed);
        let thread = std::thread::Builder::new()
            .name("meeting-track-writer".into())
            .spawn(move || {
                let result = write_track(wav, receiver, origin);
                if let Err(error) = &result {
                    log::error!("Meeting track writer failed: {error}");
                    thread_failed.store(true, Ordering::SeqCst);
                }
                result
            })
            .map_err(|error| format!("Cannot start the track writer: {error}"))?;
        Ok(Self {
            sender,
            failed,
            thread,
        })
    }

    pub fn feeder(&self) -> Feeder {
        Feeder(self.sender.clone())
    }

    /// The file can no longer be written (for example, the disk is full).
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Writes what is queued, pads the track with silence to `duration`, and
    /// closes the file. Returns the number of samples in it.
    pub fn finish(self, duration: Duration) -> Result<u64, String> {
        let total = samples_in(duration);
        // A writer that already failed has dropped its receiver.
        let _ = self.sender.send(Message::Finish { total });
        drop(self.sender);
        self.thread
            .join()
            .map_err(|_| "The track writer panicked".to_string())?
    }
}

type Wav = hound::WavWriter<BufWriter<File>>;

fn write_track(mut wav: Wav, receiver: Receiver<Message>, origin: Instant) -> Result<u64, String> {
    let mut resampler: Option<(u32, FrameResampler)> = None;
    let mut written = 0u64;
    let mut frames = Vec::new();
    let mut last_flush = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Message::Samples { data, sample_rate }) => {
                frames.clear();
                // The output device can change mid-meeting, and its rate with it.
                if resampler.as_ref().map(|(rate, _)| *rate) != Some(sample_rate) {
                    if let Some((_, mut previous)) = resampler.take() {
                        previous.finish(|frame| frames.extend_from_slice(frame));
                    }
                    resampler = Some((
                        sample_rate,
                        FrameResampler::new(
                            sample_rate as usize,
                            TARGET_SAMPLE_RATE as usize,
                            Duration::from_millis(20),
                        ),
                    ));
                }
                if let Some((_, resampler)) = resampler.as_mut() {
                    resampler.push(&data, |frame| frames.extend_from_slice(frame));
                }
                written += write_samples(&mut wav, &frames)?;
            }
            Ok(Message::Finish { total }) => {
                frames.clear();
                if let Some((_, mut resampler)) = resampler.take() {
                    resampler.finish(|frame| frames.extend_from_slice(frame));
                }
                written += write_samples(&mut wav, &frames)?;
                written += write_silence(&mut wav, total.saturating_sub(written))?;
                wav.finalize().map_err(|error| error.to_string())?;
                return Ok(written);
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The capture was dropped without finishing: keep what was written.
            Err(RecvTimeoutError::Disconnected) => {
                wav.finalize().map_err(|error| error.to_string())?;
                return Ok(written);
            }
        }
        let expected = samples_in(origin.elapsed());
        written += write_silence(&mut wav, silence_to_insert(written, expected))?;
        if last_flush.elapsed() >= HEADER_FLUSH_INTERVAL {
            wav.flush().map_err(|error| error.to_string())?;
            last_flush = Instant::now();
        }
    }
}

fn write_samples(wav: &mut Wav, samples: &[f32]) -> Result<u64, String> {
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        wav.write_sample(value).map_err(|error| error.to_string())?;
    }
    Ok(samples.len() as u64)
}

fn write_silence(wav: &mut Wav, samples: u64) -> Result<u64, String> {
    for _ in 0..samples {
        wav.write_sample(0i16).map_err(|error| error.to_string())?;
    }
    Ok(samples)
}

/// Opens the system default microphone and the default output.
pub struct DeviceCapture;

impl CaptureBackend for DeviceCapture {
    fn system_audio(&self) -> system::SystemAudioSupport {
        system::support()
    }

    fn start_mic(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String> {
        Ok(Box::new(mic::MicCapture::start(
            &dir.join(MIC_TRACK),
            origin,
        )?))
    }

    fn start_system(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String> {
        Ok(Box::new(system::SystemCapture::start(
            &dir.join(SYSTEM_TRACK),
            origin,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(sample_rate: u32, seconds: f64) -> Vec<f32> {
        let count = (f64::from(sample_rate) * seconds) as usize;
        (0..count)
            .map(|index| {
                (index as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32).sin() * 0.5
            })
            .collect()
    }

    fn read(path: &Path) -> (hound::WavSpec, Vec<i16>) {
        let reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        (spec, reader.into_samples().map(Result::unwrap).collect())
    }

    #[test]
    fn a_source_behind_the_clock_by_more_than_half_a_second_gets_silence() {
        assert_eq!(silence_to_insert(16_000, 16_000), 0);
        assert_eq!(silence_to_insert(16_000, 24_000), 0);
        assert_eq!(silence_to_insert(16_000, 24_001), 8_001);
        assert_eq!(silence_to_insert(40_000, 16_000), 0);
    }

    #[test]
    fn audio_is_resampled_to_16_khz_mono_and_padded_to_the_meeting_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MIC_TRACK);
        let writer = TrackWriter::create(&path, Instant::now()).unwrap();
        let feeder = writer.feeder();
        for chunk in sine(48_000, 1.0).chunks(480) {
            feeder.push(chunk.to_vec(), 48_000);
        }

        let written = writer.finish(Duration::from_secs(3)).unwrap();

        let (spec, samples) = read(&path);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        assert_eq!(written, 48_000);
        assert_eq!(samples.len(), 48_000);
        let loud = |range: std::ops::Range<usize>| samples[range].iter().any(|s| s.abs() > 8_000);
        assert!(loud(1_000..15_000), "the tone is in the first second");
        assert!(!loud(17_000..48_000), "the rest is silence");
    }

    #[test]
    fn a_change_of_input_rate_keeps_every_second_of_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SYSTEM_TRACK);
        let writer = TrackWriter::create(&path, Instant::now()).unwrap();
        let feeder = writer.feeder();
        for chunk in sine(48_000, 1.0).chunks(512) {
            feeder.push(chunk.to_vec(), 48_000);
        }
        for chunk in sine(44_100, 1.0).chunks(512) {
            feeder.push(chunk.to_vec(), 44_100);
        }

        let written = writer.finish(Duration::ZERO).unwrap();

        // Each resampler pads or holds back part of its last block.
        assert!(
            (31_500..34_000).contains(&written),
            "{written} samples for two seconds of audio"
        );
        assert_eq!(read(&path).1.len() as u64, written);
    }

    #[test]
    fn a_track_that_cannot_be_created_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-folder").join(MIC_TRACK);
        assert!(TrackWriter::create(&missing, Instant::now()).is_err());
    }
}
