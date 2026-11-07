//! The "Me" track: the microphone dictation uses (the one chosen in
//! settings, else the system default), on a cpal stream of its own next to
//! the dictation recorder's (Core Audio serves several clients from one
//! device). When the call app records from another input, the track follows
//! it (`call_input.rs`): always with the default, and with a microphone
//! chosen in Settings only once that one delivers nothing but silence. If the
//! device stops (unplugged, or a Bluetooth headset dropped out), it is
//! reopened, or the default input once it is gone; the writer fills the gap,
//! and the meeting warns when no reopen brings audio back.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SizedSample};

use super::call_input::{self, Look, Route, Router};
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

    fn dropped_ms(&self) -> u64 {
        self.writer.dropped_ms()
    }

    fn live(&self, tap: crate::meetings::live::LiveTap) {
        self.writer.live(tap);
    }

    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.supervisor.join();
        self.writer.finish(duration).map(|_| false)
    }
}

/// Keeps a stream on `input` open until `stop`, on the device the call app
/// uses when it uses another. The first open's outcome goes to `ready`; later
/// failures are retried.
fn supervise(
    input: Input,
    feeder: Feeder,
    stop: Arc<AtomicBool>,
    heard: Arc<Heard>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let mut ready = Some(ready);
    let mut router = Router::default();
    router.start(&first_call_devices(), input.device.is_some());
    while !stop.load(Ordering::SeqCst) {
        let broken = Arc::new(AtomicBool::new(false));
        let mut rerouted = false;
        match open_input(&input, router.route(), &feeder, &broken, &heard) {
            Ok(opened) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                let began = Instant::now();
                let mut looked = began;
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = Instant::now();
                    let silent_for = heard.silent_for(now).min(now - began);
                    if broken.load(Ordering::SeqCst) || silent_for > STALL {
                        log::warn!("Meeting microphone stopped delivering audio; reopening it");
                        // A call's device that stalls gives way to the
                        // settings instead of being reopened for good.
                        if *router.route() != Route::Settings {
                            router.failed(now);
                            rerouted = true;
                        }
                        break;
                    }
                    if now.duration_since(looked) < call_input::POLL {
                        continue;
                    }
                    looked = now;
                    let calls = call_devices();
                    let look = Look {
                        calls: &calls,
                        open: opened.name.as_deref(),
                        explicit: opened.explicit,
                        quiet: heard.quiet_for(now).min(now - began),
                    };
                    if router.observe(now, &look).is_some() {
                        log::info!("The meeting microphone follows the call app's input");
                        rerouted = true;
                        break;
                    }
                }
                drop(opened);
            }
            Err(error) if *router.route() != Route::Settings => {
                // The call's device cannot be opened: the settings decide.
                log::warn!("Cannot open the call app's microphone: {error}");
                router.failed(Instant::now());
                rerouted = true;
            }
            Err(error) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(error));
                    return;
                }
                log::warn!("Cannot reopen the meeting microphone: {error}");
            }
        }
        if !rerouted && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(REOPEN_DELAY);
        }
    }
}

/// The first look at the call apps' devices, before the microphone opens.
/// The system is read on a thread of its own and given a second: a driver
/// that hangs must not delay the start of the meeting.
fn first_call_devices() -> Vec<String> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = send.send(call_devices());
    });
    receive
        .recv_timeout(Duration::from_secs(1))
        .unwrap_or_default()
}

/// The input devices the call apps record from now; empty where the system
/// does not tell.
fn call_devices() -> Vec<String> {
    call_input::inputs_now()
        .map(|users| call_input::call_devices(&users))
        .unwrap_or_default()
}

/// The device named `wanted` among `devices`; `None` means the default.
fn named_input<D>(devices: Vec<(String, D)>, wanted: Option<&str>) -> Option<D> {
    let wanted = wanted?;
    devices
        .into_iter()
        .find(|(name, _)| name == wanted)
        .map(|(_, device)| device)
}

/// A running stream and what it records from.
struct Opened {
    /// Held so that the stream keeps running until `Opened` is dropped.
    _stream: cpal::Stream,
    /// The device's name, when cpal tells.
    name: Option<String>,
    /// The device is the one chosen in Settings.
    explicit: bool,
}

/// The input devices by name, empty when they cannot be listed.
fn input_devices() -> Vec<(String, cpal::Device)> {
    match crate::audio_toolkit::list_input_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(|info| (info.name, info.device))
            .collect(),
        Err(error) => {
            log::warn!("Cannot list the microphones: {error}");
            Vec::new()
        }
    }
}

/// The device `route` names, with whether it is the one chosen in Settings.
/// A call app's device that cannot be found, or told apart from another of
/// the same name, gives way to the settings.
fn resolve_input(input: &Input, route: &Route) -> Result<(cpal::Device, bool), String> {
    if let Route::Call(wanted) = route {
        match call_input::unique_named(input_devices(), wanted) {
            Some(device) => return Ok((device, false)),
            None => log::warn!(
                "The call app's microphone is not available here; the meeting uses its usual one"
            ),
        }
    }
    let wanted = input.device.as_deref();
    let named = wanted.and_then(|_| {
        let found = named_input(input_devices(), wanted);
        if found.is_none() {
            log::warn!("The chosen microphone is missing; the meeting uses the default");
        }
        found
    });
    match named {
        Some(device) => Ok((device, true)),
        None => cpal::default_host()
            .default_input_device()
            .map(|device| (device, false))
            .ok_or_else(|| "No input device found".to_string()),
    }
}

fn open_input(
    input: &Input,
    route: &Route,
    feeder: &Feeder,
    broken: &Arc<AtomicBool>,
    heard: &Arc<Heard>,
) -> Result<Opened, String> {
    let (device, explicit) = resolve_input(input, route)?;
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
    Ok(Opened {
        _stream: stream,
        name: device.name().ok(),
        explicit,
    })
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
            heard.mark(Instant::now(), peak(&samples));
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

/// When the microphone last delivered audio, across every reopen, and when it
/// last delivered more than silence. Digital zeros count as audio for the
/// first: a muted input gives them, and so does a denied permission, which
/// the coordinator reads from macOS instead. The second tells a chosen
/// microphone that gives nothing from a quiet room.
struct Heard {
    epoch: Instant,
    last_ms: AtomicU64,
    loud_ms: AtomicU64,
}

impl Heard {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            last_ms: AtomicU64::new(0),
            loud_ms: AtomicU64::new(0),
        }
    }

    /// A callback delivered samples whose loudest was `peak`.
    fn mark(&self, now: Instant, peak: f32) {
        let ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
        self.last_ms.store(ms, Ordering::SeqCst);
        if peak > call_input::SILENT_PEAK {
            self.loud_ms.store(ms, Ordering::SeqCst);
        }
    }

    fn silent_for(&self, now: Instant) -> Duration {
        let last = self.epoch + Duration::from_millis(self.last_ms.load(Ordering::SeqCst));
        now.saturating_duration_since(last)
    }

    /// How long the microphone has delivered nothing above silence.
    fn quiet_for(&self, now: Instant) -> Duration {
        let last = self.epoch + Duration::from_millis(self.loud_ms.load(Ordering::SeqCst));
        now.saturating_duration_since(last)
    }

    fn lost(&self, now: Instant) -> bool {
        self.silent_for(now) > LOST
    }
}

/// The loudest sample of `samples`.
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0, |loudest, sample| loudest.max(sample.abs()))
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

        heard.mark(start + Duration::from_secs(10), 0.0);
        assert!(!heard.lost(start + Duration::from_secs(10) + LOST));
        assert!(heard.lost(start + Duration::from_secs(11) + LOST));
    }

    #[test]
    fn a_microphone_muted_from_the_start_is_not_lost() {
        // A headset mute switch or a mute utility sets the gain to 0: the
        // callbacks keep coming, all zeros.
        let start = Instant::now();
        let muted = Heard::new(start);
        for second in 1..=10 {
            muted.mark(start + Duration::from_secs(second), 0.0);
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

    #[test]
    fn a_microphone_counts_as_quiet_until_it_delivers_more_than_silence() {
        let start = Instant::now();
        let heard = Heard::new(start);
        let second = Duration::from_secs(1);
        // Zeros and the faintest hiss are silence, however many callbacks.
        for n in 1..=12 {
            heard.mark(start + n * second, call_input::SILENT_PEAK);
        }
        assert_eq!(heard.quiet_for(start + 12 * second), 12 * second);
        assert!(!heard.lost(start + 12 * second), "it still delivers");

        heard.mark(start + 13 * second, 0.2);
        assert_eq!(heard.quiet_for(start + 15 * second), 2 * second);
    }

    #[test]
    fn the_loudest_sample_ignores_its_sign() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
    }
}
