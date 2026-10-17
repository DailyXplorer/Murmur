//! Meeting audio capture: the microphone and the system output, each streamed
//! to its own 16 kHz mono WAV on disk, so a three-hour meeting never sits in
//! memory.
//!
//! Both tracks share one timeline that starts when the meeting starts. Each
//! chunk carries the time it was captured, and its writer places it there:
//! the first chunk after the start latency, later ones after a gap (a device
//! was unplugged, the output device changed) or drift. So a sample index
//! means the same moment in both files.

pub mod mic;
pub mod system;

use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::coordinator::{CaptureBackend, Track};
use super::paths::{MIC_TRACK, SYSTEM_TRACK};
use super::wav_recovery::repair_wav;
use crate::audio_toolkit::audio::FrameResampler;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;

const RATE: u64 = TARGET_SAMPLE_RATE as u64;
/// Seconds a track may sit off its capture timestamps before it is realigned.
/// Callback jitter stays well below it; OBS smooths audio timestamps with a
/// similar window.
const SYNC_TOLERANCE: f64 = 0.05;
const BYTES_PER_SAMPLE: u64 = 2;
/// How often the header is rewritten, which bounds what a crash can lose
/// track of (`wav_recovery` restores the rest from the file length).
const HEADER_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Audio callbacks deliver every few milliseconds; this holds several seconds.
const QUEUE_CHUNKS: usize = 1024;

/// Seconds of silence to insert (positive) or of audio to drop (negative) so
/// a chunk captured at `start` lands there on a track already `placed`
/// seconds long. The first chunk is placed exactly; later ones only once the
/// track has drifted past the tolerance, in either direction.
fn correction(placed: f64, start: f64) -> f64 {
    let offset = start - placed;
    if placed == 0.0 || offset.abs() > SYNC_TOLERANCE {
        offset
    } else {
        0.0
    }
}

fn samples_in(duration: Duration) -> u64 {
    (duration.as_secs_f64() * RATE as f64).round() as u64
}

enum Message {
    Samples {
        data: Vec<f32>,
        sample_rate: u32,
        /// When the last sample of `data` was captured.
        captured: Instant,
    },
    Finish {
        total: u64,
    },
}

/// Hands mono audio from a capture callback to its writer. It never blocks:
/// a full queue drops the chunk, and the gap is filled with silence.
#[derive(Clone)]
pub struct Feeder(SyncSender<Message>);

impl Feeder {
    /// Called from the capture callback as the chunk arrives, which stamps it.
    pub fn push(&self, data: Vec<f32>, sample_rate: u32) {
        self.push_at(data, sample_rate, Instant::now());
    }

    fn push_at(&self, data: Vec<f32>, sample_rate: u32, captured: Instant) {
        if sample_rate > 0 && !data.is_empty() {
            let _ = self.0.try_send(Message::Samples {
                data,
                sample_rate,
                captured,
            });
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
        let path = path.to_path_buf();
        let failed = Arc::new(AtomicBool::new(false));
        let thread_failed = Arc::clone(&failed);
        let thread = std::thread::Builder::new()
            .name("meeting-track-writer".into())
            .spawn(move || {
                let result = write_track(wav, &path, receiver, origin);
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

    /// Writes what is queued, pads or cuts the track to exactly `duration`,
    /// and closes the file. Returns the number of samples in it.
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

fn write_track(
    mut wav: Wav,
    path: &Path,
    receiver: Receiver<Message>,
    origin: Instant,
) -> Result<u64, String> {
    let mut resampler: Option<(u32, FrameResampler)> = None;
    let mut written = 0u64;
    // Seconds handed to the resampler, silence included: where the next
    // sample lands on the meeting timeline.
    let mut placed = 0f64;
    let mut last_flush = Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Message::Samples {
                data,
                sample_rate,
                captured,
            }) => {
                // The output device can change mid-meeting, and its rate with it.
                if resampler.as_ref().map(|(rate, _)| *rate) != Some(sample_rate) {
                    if let Some((_, mut previous)) = resampler.take() {
                        let mut frames = Vec::new();
                        previous.finish(|frame| frames.extend_from_slice(frame));
                        written += write_samples(&mut wav, &frames)?;
                        placed = written as f64 / RATE as f64;
                    }
                }
                let (_, resampler) = resampler.get_or_insert_with(|| {
                    (
                        sample_rate,
                        FrameResampler::new(
                            sample_rate as usize,
                            TARGET_SAMPLE_RATE as usize,
                            Duration::from_millis(20),
                        ),
                    )
                });
                let rate = f64::from(sample_rate);
                let end = captured.saturating_duration_since(origin).as_secs_f64();
                let start = (end - data.len() as f64 / rate).max(0.0);
                let offset = correction(placed, start);
                let mut audio = &data[..];
                if offset > 0.0 {
                    let silence = (offset * rate).round() as usize;
                    written += feed_silence(resampler, &mut wav, silence)?;
                    placed += silence as f64 / rate;
                } else if offset < 0.0 {
                    let ahead = (-offset * rate).round() as usize;
                    audio = &audio[ahead.min(audio.len())..];
                }
                written += feed(resampler, &mut wav, audio)?;
                placed += audio.len() as f64 / rate;
            }
            Ok(Message::Finish { total }) => {
                let mut frames = Vec::new();
                if let Some((_, mut resampler)) = resampler.take() {
                    resampler.finish(|frame| frames.extend_from_slice(frame));
                }
                let room = total.saturating_sub(written) as usize;
                written += write_samples(&mut wav, &frames[..frames.len().min(room)])?;
                written += write_silence(&mut wav, total.saturating_sub(written))?;
                wav.finalize().map_err(|error| error.to_string())?;
                if written > total {
                    truncate(path, written, total)?;
                    written = total;
                }
                return Ok(written);
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The capture was dropped without finishing: keep what was written.
            Err(RecvTimeoutError::Disconnected) => {
                wav.finalize().map_err(|error| error.to_string())?;
                return Ok(written);
            }
        }
        if last_flush.elapsed() >= HEADER_FLUSH_INTERVAL {
            wav.flush().map_err(|error| error.to_string())?;
            last_flush = Instant::now();
        }
    }
}

fn feed(resampler: &mut FrameResampler, wav: &mut Wav, samples: &[f32]) -> Result<u64, String> {
    let mut frames = Vec::new();
    resampler.push(samples, |frame| frames.extend_from_slice(frame));
    write_samples(wav, &frames)
}

/// Silence goes through the resampler too, so it lands in order with the
/// audio the resampler still holds. Blocks keep a long gap out of memory.
fn feed_silence(
    resampler: &mut FrameResampler,
    wav: &mut Wav,
    mut count: usize,
) -> Result<u64, String> {
    const BLOCK: [f32; 4096] = [0.0; 4096];
    let mut written = 0;
    while count > 0 {
        let take = count.min(BLOCK.len());
        written += feed(resampler, wav, &BLOCK[..take])?;
        count -= take;
    }
    Ok(written)
}

/// Cuts a closed track back to `total` samples: audio captured after the
/// meeting ended, while the other track was still stopping.
fn truncate(path: &Path, written: u64, total: u64) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let len = file.metadata().map_err(|error| error.to_string())?.len();
    file.set_len(len - (written - total) * BYTES_PER_SAMPLE)
        .map_err(|error| error.to_string())?;
    drop(file);
    repair_wav(path).map_err(|error| error.to_string())?;
    Ok(())
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

/// Opens the microphone dictation uses and the default output.
pub struct DeviceCapture(pub tauri::AppHandle);

impl CaptureBackend for DeviceCapture {
    fn system_audio(&self) -> system::SystemAudioSupport {
        system::support()
    }

    fn start_mic(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String> {
        let device =
            crate::managers::audio::preferred_microphone(&crate::settings::get_settings(&self.0));
        Ok(Box::new(mic::MicCapture::start(
            &dir.join(MIC_TRACK),
            origin,
            device,
        )?))
    }

    fn start_system(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String> {
        Ok(Box::new(system::SystemCapture::start(
            &dir.join(SYSTEM_TRACK),
            origin,
        )?))
    }

    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        free_bytes(dir)
    }
}

/// The space an unprivileged process may still write on the volume of
/// `path`. Space macOS could purge on demand is not counted, so this errs on
/// the low side.
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` is written before it is read.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded, so it filled `stats`.
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Some(stats.f_bavail as u64 * stats.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_is_read_for_a_folder_and_unknown_for_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_bytes(dir.path()).is_some_and(|free| free > 0));
        assert_eq!(free_bytes(&dir.path().join("missing")), None);
    }

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

    /// Pushes `samples` in chunks stamped as a device delivering
    /// `device_rate` samples per second of capture time would, from `from`
    /// seconds after `origin`. Returns where the next chunk would start.
    fn deliver(
        feeder: &Feeder,
        origin: Instant,
        samples: &[f32],
        chunk: usize,
        sample_rate: u32,
        device_rate: f64,
        from: f64,
    ) -> f64 {
        let mut at = from;
        for data in samples.chunks(chunk) {
            at += data.len() as f64 / device_rate;
            feeder.push_at(
                data.to_vec(),
                sample_rate,
                origin + Duration::from_secs_f64(at),
            );
        }
        at
    }

    fn onset(samples: &[i16]) -> usize {
        samples.iter().position(|s| s.abs() > 8_000).unwrap()
    }

    #[test]
    fn a_track_off_its_capture_time_is_realigned_in_either_direction() {
        assert_eq!(correction(0.0, 0.3), 0.3, "the first chunk lands exactly");
        assert_eq!(correction(1.0, 1.04), 0.0);
        assert_eq!(correction(1.0, 0.96), 0.0);
        assert!((correction(1.0, 1.2) - 0.2).abs() < 1e-9, "behind: silence");
        assert!((correction(1.0, 0.8) + 0.2).abs() < 1e-9, "ahead: trimmed");
    }

    #[test]
    fn a_track_whose_first_audio_arrives_late_starts_with_that_much_silence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SYSTEM_TRACK);
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        deliver(
            &writer.feeder(),
            origin,
            &sine(16_000, 0.5),
            160,
            16_000,
            16_000.0,
            0.3,
        );

        writer.finish(Duration::from_secs(1)).unwrap();

        let (_, samples) = read(&path);
        assert!(samples[..4_800].iter().all(|s| *s == 0));
        assert!(onset(&samples) - 4_800 < 16, "the tone starts at 300 ms");
        assert_eq!(samples.len(), 16_000);
    }

    #[test]
    fn a_track_ahead_of_its_capture_time_is_trimmed_and_one_behind_is_padded() {
        for (device_rate, label) in [(17_600.0, "fast"), (14_400.0, "slow")] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(MIC_TRACK);
            let origin = Instant::now();
            let writer = TrackWriter::create(&path, origin).unwrap();
            let feeder = writer.feeder();
            // A device clock 10% off delivers 1.1 s or 0.9 s of samples
            // per second; a marker tone at 1 s must still land at 1 s.
            let quiet = vec![0.01; device_rate as usize];
            let at = deliver(&feeder, origin, &quiet, 160, 16_000, device_rate, 0.0);
            deliver(
                &feeder,
                origin,
                &sine(16_000, 0.2),
                160,
                16_000,
                16_000.0,
                at,
            );

            writer.finish(Duration::from_secs(2)).unwrap();

            let (_, samples) = read(&path);
            let marker = onset(&samples) as i64;
            assert!(
                (marker - 16_000).abs() <= 800 + 16,
                "{label} device: the 1 s marker is at sample {marker}"
            );
            assert_eq!(samples.len(), 32_000);
        }
    }

    #[test]
    fn finishing_never_leaves_audio_past_the_meeting_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SYSTEM_TRACK);
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        // The other track was still stopping: this one kept capturing.
        let tail = sine(48_000, 2.0);
        deliver(&writer.feeder(), origin, &tail, 480, 48_000, 48_000.0, 0.0);

        let written = writer.finish(Duration::from_secs(1)).unwrap();

        assert_eq!(written, 16_000);
        let (_, samples) = read(&path);
        assert_eq!(samples.len(), 16_000);
    }

    #[test]
    fn audio_is_resampled_to_16_khz_mono_and_padded_to_the_meeting_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MIC_TRACK);
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        let feeder = writer.feeder();
        deliver(
            &feeder,
            origin,
            &sine(48_000, 1.0),
            480,
            48_000,
            48_000.0,
            0.0,
        );

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
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        let feeder = writer.feeder();
        let at = deliver(
            &feeder,
            origin,
            &sine(48_000, 1.0),
            512,
            48_000,
            48_000.0,
            0.0,
        );
        deliver(
            &feeder,
            origin,
            &sine(44_100, 1.0),
            512,
            44_100,
            44_100.0,
            at,
        );

        writer.finish(Duration::from_secs(3)).unwrap();

        // Each resampler pads or holds back part of its last block.
        let samples = read(&path).1;
        let last = samples.iter().rposition(|s| s.abs() > 8_000).unwrap();
        assert!(
            (31_000..34_000).contains(&last),
            "two seconds of audio end at sample {last}"
        );
        assert_eq!(samples.len(), 48_000);
    }

    #[test]
    fn a_track_that_cannot_be_created_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-folder").join(MIC_TRACK);
        assert!(TrackWriter::create(&missing, Instant::now()).is_err());
    }
}
