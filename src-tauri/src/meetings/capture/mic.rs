//! The "Me" track: the microphone dictation uses (the one chosen in
//! settings, else the system default), on a cpal stream of its own next to
//! the dictation recorder's (Core Audio serves several clients from one
//! device). If the device stops (unplugged, or a Bluetooth headset dropped
//! out), it is reopened, or the default input once it is gone; the writer
//! fills the gap.

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
    supervisor: JoinHandle<()>,
}

impl MicCapture {
    /// Records from the input named `device`, or the system default when it
    /// is `None` or missing. Fails when the microphone cannot be opened at
    /// all.
    pub fn start(path: &Path, origin: Instant, device: Option<String>) -> Result<Self, String> {
        let writer = TrackWriter::create(path, origin)?;
        let feeder = writer.feeder();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_sender, ready) = mpsc::sync_channel(1);
        let supervisor_stop = Arc::clone(&stop);
        let supervisor = std::thread::Builder::new()
            .name("meeting-mic".into())
            .spawn(move || supervise(device, feeder, supervisor_stop, ready_sender))
            .map_err(|error| format!("Cannot start the microphone thread: {error}"))?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                writer,
                stop,
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
    /// filled, so it never counts as lost.
    fn interrupted(&self) -> bool {
        false
    }

    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.supervisor.join();
        self.writer.finish(duration).map(|_| false)
    }
}

/// Keeps a stream on the input named `device` (or the default) open until
/// `stop`. The first open's outcome goes to `ready`; later failures are
/// retried.
fn supervise(
    device: Option<String>,
    feeder: Feeder,
    stop: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let mut ready = Some(ready);
    let epoch = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let broken = Arc::new(AtomicBool::new(false));
        let last_audio_ms = Arc::new(AtomicU64::new(epoch.elapsed().as_millis() as u64));
        match open_input(device.as_deref(), &feeder, &broken, &last_audio_ms, epoch) {
            Ok(stream) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let silent_for = (epoch.elapsed().as_millis() as u64)
                        .saturating_sub(last_audio_ms.load(Ordering::SeqCst));
                    if broken.load(Ordering::SeqCst) || silent_for > STALL.as_millis() as u64 {
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
    wanted: Option<&str>,
    feeder: &Feeder,
    broken: &Arc<AtomicBool>,
    last_audio_ms: &Arc<AtomicU64>,
    epoch: Instant,
) -> Result<cpal::Stream, String> {
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
            build::<f32>(&device, &config, feeder, broken, last_audio_ms, epoch)
        }
        cpal::SampleFormat::I16 => {
            build::<i16>(&device, &config, feeder, broken, last_audio_ms, epoch)
        }
        cpal::SampleFormat::I32 => {
            build::<i32>(&device, &config, feeder, broken, last_audio_ms, epoch)
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
    feeder: &Feeder,
    broken: &Arc<AtomicBool>,
    last_audio_ms: &Arc<AtomicU64>,
    epoch: Instant,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: Sample + SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let sample_rate = config.sample_rate.0;
    let feeder = feeder.clone();
    let last_audio_ms = Arc::clone(last_audio_ms);
    let broken = Arc::clone(broken);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mono = data
                .chunks_exact(channels)
                .map(|frame| {
                    frame
                        .iter()
                        .map(|sample| sample.to_sample::<f32>())
                        .sum::<f32>()
                        / channels as f32
                })
                .collect();
            feeder.push(mono, sample_rate);
            last_audio_ms.store(epoch.elapsed().as_millis() as u64, Ordering::SeqCst);
        },
        move |error| {
            log::error!("Meeting microphone stream error: {error}");
            broken.store(true, Ordering::SeqCst);
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
