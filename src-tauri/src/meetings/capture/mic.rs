//! The "Me" track: the microphone dictation uses (the one chosen in
//! settings, else the system default), on a cpal stream of its own next to
//! the dictation recorder's (Core Audio serves several clients from one
//! device). If the device stops (unplugged, or a Bluetooth headset dropped
//! out), it is reopened, or the default input once it is gone; the writer
//! fills the gap, and the meeting warns when no reopen brings audio back.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SizedSample};

use super::{Feeder, TrackWriter};
use crate::meetings::coordinator::Track;

/// Core Audio delivers input every few milliseconds; this much silence from
/// the callback means the device is gone even if cpal reported nothing.
const STALL: Duration = Duration::from_secs(3);
const REOPEN_DELAY: Duration = Duration::from_secs(1);

pub struct MicCapture {
    writer: TrackWriter,
    stop: Arc<AtomicBool>,
    heard: Arc<Heard>,
    supervisor: JoinHandle<()>,
}

/// Which microphone records, and how.
struct Input {
    /// The input named so, or the system default when `None` or missing.
    device: Option<String>,
    /// The input channel chosen in settings; `None` averages them all.
    channel: Option<usize>,
}

impl MicCapture {
    /// Records from the input named `device`, or the system default when it
    /// is `None` or missing, keeping only `channel` when the device has it,
    /// as dictation does. Fails when the microphone cannot be opened at all.
    pub fn start(
        path: &Path,
        origin: Instant,
        device: Option<String>,
        channel: Option<u16>,
    ) -> Result<Self, String> {
        let writer = TrackWriter::create(path, origin)?;
        let feeder = writer.feeder();
        let stop = Arc::new(AtomicBool::new(false));
        let heard = Arc::new(Heard::new(Instant::now()));
        let (ready_sender, ready) = mpsc::sync_channel(1);
        let input = Input {
            device,
            channel: channel.map(usize::from),
        };
        let (supervisor_stop, supervisor_heard) = (Arc::clone(&stop), Arc::clone(&heard));
        let supervisor = std::thread::Builder::new()
            .name("meeting-mic".into())
            .spawn(move || {
                supervise(
                    input,
                    feeder,
                    supervisor_stop,
                    supervisor_heard,
                    ready_sender,
                )
            })
            .map_err(|error| format!("Cannot start the microphone thread: {error}"))?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                writer,
                stop,
                heard,
                supervisor,
            }),
            Ok(Err(error)) => {
                let _ = supervisor.join();
                Err(error)
            }
            Err(_) => {
                let _ = supervisor.join();
                Err("The microphone thread stopped".to_string())
            }
        }
    }
}

impl Track for MicCapture {
    fn failed(&self) -> bool {
        self.writer.failed()
    }

    /// A stalled microphone is reopened in the background and the gap is
    /// filled; it counts as lost once no reopen brought audio back.
    fn interrupted(&self) -> bool {
        self.heard.lost(Instant::now())
    }

    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.supervisor.join();
        self.writer.finish(duration).map(|_| false)
    }
}

/// Keeps a stream on `input` open until `stop`. The first open's outcome
/// goes to `ready`; later failures are retried.
fn supervise(
    input: Input,
    feeder: Feeder,
    stop: Arc<AtomicBool>,
    heard: Arc<Heard>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let mut ready = Some(ready);
    while !stop.load(Ordering::SeqCst) {
        let broken = Arc::new(AtomicBool::new(false));
        match open_input(&input, &feeder, &broken, &heard) {
            Ok(stream) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                let opened = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = Instant::now();
                    let silent_for = heard.silent_for(now).min(now - opened);
                    if broken.load(Ordering::SeqCst) || silent_for > STALL {
                        log::warn!("Meeting microphone stopped delivering audio; reopening it");
                        break;
                    }
                }
                drop(stream);
            }
            Err(error) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(error));
                    return;
                }
                log::warn!("Cannot reopen the meeting microphone: {error}");
            }
        }
        if !stop.load(Ordering::SeqCst) {
            std::thread::sleep(REOPEN_DELAY);
        }
    }
}

/// The device named `wanted` among `devices`; `None` means the default.
fn named_input<D>(devices: Vec<(String, D)>, wanted: Option<&str>) -> Option<D> {
    let wanted = wanted?;
    devices
        .into_iter()
        .find(|(name, _)| name == wanted)
        .map(|(_, device)| device)
}

fn open_input(
    input: &Input,
    feeder: &Feeder,
    broken: &Arc<AtomicBool>,
    heard: &Arc<Heard>,
) -> Result<cpal::Stream, String> {
    let wanted = input.device.as_deref();
    let named = wanted.and_then(|_| {
        let devices = crate::audio_toolkit::list_input_devices()
            .map_err(|error| log::warn!("Cannot list the microphones: {error}"))
            .ok()?;
        let found = named_input(
            devices
                .into_iter()
                .map(|info| (info.name, info.device))
                .collect(),
            wanted,
        );
        if found.is_none() {
            log::warn!("The chosen microphone is missing; the meeting uses the default");
        }
        found
    });
    let device = match named {
        Some(device) => device,
        None => cpal::default_host()
            .default_input_device()
            .ok_or_else(|| "No input device found".to_string())?,
    };
    let config = device
        .default_input_config()
        .map_err(|error| format!("Cannot read the microphone format: {error}"))?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let stream = match format {
        cpal::SampleFormat::F32 => {
            build::<f32>(&device, &config, input.channel, feeder, broken, heard)
        }
        cpal::SampleFormat::I16 => {
            build::<i16>(&device, &config, input.channel, feeder, broken, heard)
        }
        cpal::SampleFormat::I32 => {
            build::<i32>(&device, &config, input.channel, feeder, broken, heard)
        }
        other => return Err(format!("Unsupported microphone sample format {other:?}")),
    }
    .map_err(|error| format!("Cannot open the microphone: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("Cannot start the microphone: {error}"))?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channel: Option<usize>,
    feeder: &Feeder,
    broken: &Arc<AtomicBool>,
    heard: &Arc<Heard>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let sample_rate = config.sample_rate.0;
    let feeder = feeder.clone();
    let heard = Arc::clone(heard);
    let broken = Arc::clone(broken);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let samples = mono(data, channels, channel);
            heard.mark(Instant::now(), &samples);
            feeder.push(samples, sample_rate);
        },
        move |error| {
            log::error!("Meeting microphone stream error: {error}");
            broken.store(true, Ordering::SeqCst);
        },
        None,
    )
}

/// Without audio this long, reopening the microphone did not bring it back.
const LOST: Duration = Duration::from_secs(6);

/// When the microphone last delivered audio, across every reopen, and
/// whether any of it was more than digital zeros.
struct Heard {
    epoch: Instant,
    last_ms: AtomicU64,
    voiced: AtomicBool,
}

impl Heard {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            last_ms: AtomicU64::new(0),
            voiced: AtomicBool::new(false),
        }
    }

    fn mark(&self, now: Instant, samples: &[f32]) {
        let ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
        self.last_ms.store(ms, Ordering::SeqCst);
        if samples.iter().any(|sample| *sample != 0.0) {
            self.voiced.store(true, Ordering::Relaxed);
        }
    }

    fn silent_for(&self, now: Instant) -> Duration {
        let last = self.epoch + Duration::from_millis(self.last_ms.load(Ordering::SeqCst));
        now.saturating_duration_since(last)
    }

    /// No audio for a while, or only digital zeros since the start: what
    /// macOS delivers when the microphone permission is denied, since a
    /// working microphone always has a noise floor.
    fn lost(&self, now: Instant) -> bool {
        let never_voiced = !self.voiced.load(Ordering::Relaxed)
            && now.saturating_duration_since(self.epoch) > LOST;
        self.silent_for(now) > LOST || never_voiced
    }
}

/// One sample per frame of interleaved `data`: the input `channel` when the
/// device has it, else the average of every input, as dictation records.
fn mono<T>(data: &[T], channels: usize, channel: Option<usize>) -> Vec<f32>
where
    T: Sample,
    f32: cpal::FromSample<T>,
{
    let frames = data.chunks_exact(channels);
    match channel.filter(|channel| *channel < channels) {
        Some(channel) => frames
            .map(|frame| frame[channel].to_sample::<f32>())
            .collect(),
        None => frames
            .map(|frame| {
                frame
                    .iter()
                    .map(|sample| sample.to_sample::<f32>())
                    .sum::<f32>()
                    / channels as f32
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_channel_chosen_in_settings_records_the_meeting() {
        // Two inputs, the voice on the first.
        let frames = [0.8f32, 0.0, 0.6, 0.0];
        assert_eq!(mono(&frames, 2, Some(0)), [0.8, 0.6]);
        assert_eq!(mono(&frames, 2, Some(1)), [0.0, 0.0]);
        // No choice, or one the device does not have: all inputs averaged.
        assert_eq!(mono(&frames, 2, None), [0.4, 0.3]);
        assert_eq!(mono(&frames, 2, Some(5)), [0.4, 0.3]);
    }

    #[test]
    fn a_microphone_silent_past_a_reopen_counts_as_lost() {
        let start = Instant::now();
        let heard = Heard::new(start);
        assert!(!heard.lost(start + Duration::from_secs(1)));
        assert!(
            heard.lost(start + LOST + Duration::from_millis(1)),
            "never heard"
        );

        heard.mark(start + Duration::from_secs(10), &[0.01, -0.02]);
        assert!(!heard.lost(start + Duration::from_secs(10) + LOST));
        assert!(heard.lost(start + Duration::from_secs(11) + LOST));
    }

    #[test]
    fn a_microphone_giving_only_digital_zeros_counts_as_lost() {
        // What macOS delivers when the microphone permission is denied.
        let start = Instant::now();
        let denied = Heard::new(start);
        for second in 1..=10 {
            denied.mark(start + Duration::from_secs(second), &[0.0; 4]);
        }
        assert!(!denied.lost(start + LOST));
        assert!(denied.lost(start + Duration::from_secs(10)));

        // A real microphone always has a noise floor; a later mute is fine.
        let muted = Heard::new(start);
        muted.mark(start + Duration::from_secs(1), &[0.0, 0.0001]);
        for second in 2..=10 {
            muted.mark(start + Duration::from_secs(second), &[0.0; 4]);
        }
        assert!(!muted.lost(start + Duration::from_secs(10)));
    }

    fn devices() -> Vec<(String, &'static str)> {
        vec![
            ("MacBook Pro Microphone".to_string(), "built-in"),
            ("USB Headset".to_string(), "headset"),
        ]
    }

    #[test]
    fn the_microphone_chosen_in_settings_records_the_meeting() {
        assert_eq!(named_input(devices(), Some("USB Headset")), Some("headset"));
    }

    #[test]
    fn the_system_default_records_without_a_choice_or_once_the_choice_is_gone() {
        assert_eq!(named_input(devices(), None), None);
        assert_eq!(named_input(devices(), Some("Unplugged Mic")), None);
    }
}
