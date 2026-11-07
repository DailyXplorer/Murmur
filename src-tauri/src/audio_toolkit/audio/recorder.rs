use std::{
    io::Error,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Sample, SizedSample,
};

use crate::audio_toolkit::{
    audio::{AudioVisualiser, FrameResampler},
    constants,
    gain::{frame_levels, FRAME},
    release_tail::{self, ReleaseTail, Step, TailConfig},
};

enum Cmd {
    Start(Instant, mpsc::Sender<()>),
    Stop(mpsc::Sender<Capture>, Option<ReleaseTailRequest>),
    Shutdown,
}

/// A stop that keeps recording for a moment after it is asked (see
/// `release_tail`). Only a user's release asks for one: a cancel does not.
pub struct ReleaseTailRequest {
    pub config: TailConfig,
    /// True once the tail must end at once, as a cancel does.
    pub abort: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// What a stop hands back.
#[derive(Default)]
pub struct Capture {
    pub samples: Vec<f32>,
    /// The microphone went away before the stop, so the samples end where
    /// it did.
    pub mic_lost: bool,
}

enum AudioChunk {
    Samples(Vec<f32>),
    EndOfStream,
}

/// Core Audio calls the input callback every few milliseconds, recording or
/// not; this long without a call means the device is gone even if cpal
/// reported nothing. The meeting microphone uses the same bound.
const STALL: Duration = Duration::from_secs(3);

/// What an open stream's callbacks report about it. Each open gets its own.
struct StreamHealth {
    /// Set by cpal when the stream can no longer capture.
    failed: AtomicBool,
    /// Set when a finished clip held nothing but digital zeros: a warm stream
    /// a call app took the device from can go on delivering only zeros.
    delivered_only_zeros: AtomicBool,
    opened: Instant,
    /// How long after `opened` the input callback last ran, in milliseconds.
    last_callback_ms: AtomicU64,
}

impl StreamHealth {
    fn new(opened: Instant) -> Self {
        Self {
            failed: AtomicBool::new(false),
            delivered_only_zeros: AtomicBool::new(false),
            opened,
            last_callback_ms: AtomicU64::new(0),
        }
    }

    fn mark(&self, now: Instant) {
        let ms = now.saturating_duration_since(self.opened).as_millis() as u64;
        self.last_callback_ms.store(ms, Ordering::Relaxed);
    }

    /// Counted from the last mark, or from `opened` before any.
    fn stalled(&self, now: Instant) -> bool {
        let last =
            self.opened + Duration::from_millis(self.last_callback_ms.load(Ordering::Relaxed));
        now.saturating_duration_since(last) > STALL
    }
}

pub struct AudioRecorder {
    device: Option<Device>,
    cmd_tx: Option<mpsc::Sender<Cmd>>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    level_cb: Option<Arc<dyn Fn(Vec<f32>) + Send + Sync + 'static>>,
    selected_channel: Option<usize>,
    /// Preferred stream config cached per device name. The two HAL property
    /// queries in `get_preferred_config` cost ~40-85ms per open (worse on
    /// USB/Bluetooth), which lands on the keypress->capture path in on-demand
    /// mode. Keyed by name so a system-default change misses naturally;
    /// cleared whenever an open fails so a stale rate/format self-heals on the
    /// caller's retry.
    config_cache: Arc<Mutex<Option<(String, cpal::SupportedStreamConfig)>>>,
    /// The open stream's health, `None` while closed.
    health: Option<Arc<StreamHealth>>,
    /// When a zero clip last asked for a reopen. A muted input delivers zeros
    /// too and a rebuild cannot cure that, so the request is not repeated
    /// every dictation; it is retried after `ZERO_REOPEN_EVERY`, for a device
    /// a call app still held at the first try.
    last_zero_reopen: Mutex<Option<Instant>>,
    /// The name of the system default the open stream was opened on, `None`
    /// for a named device. cpal binds such a stream to the device that was
    /// the default then and reports no disconnect for it.
    default_name: Option<String>,
}

impl AudioRecorder {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(AudioRecorder {
            device: None,
            cmd_tx: None,
            worker_handle: None,
            level_cb: None,
            selected_channel: None,
            config_cache: Arc::new(Mutex::new(None)),
            health: None,
            last_zero_reopen: Mutex::new(None),
            default_name: None,
        })
    }

    pub fn with_level_callback<F>(mut self, cb: F) -> Self
    where
        F: Fn(Vec<f32>) + Send + Sync + 'static,
    {
        self.level_cb = Some(Arc::new(cb));
        self
    }

    pub fn with_selected_channel(mut self, channel: Option<u16>) -> Self {
        self.set_selected_channel(channel);
        self
    }

    pub fn set_selected_channel(&mut self, channel: Option<u16>) {
        self.selected_channel = channel.map(usize::from);
    }

    pub fn open(&mut self, device: Option<Device>) -> Result<(), Box<dyn std::error::Error>> {
        if self.worker_handle.is_some() {
            if !self.needs_reopen() {
                return Ok(());
            }
            log::warn!("Capture stream failed; rebuilding microphone stream");
            let _ = self.close();
        }

        let (sample_tx, sample_rx) = mpsc::channel::<AudioChunk>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let host = cpal::default_host();
        let opened_on_default = device.is_none();
        let device = match device {
            Some(dev) => dev,
            None => host
                .default_input_device()
                .ok_or_else(|| Error::new(std::io::ErrorKind::NotFound, "No input device found"))?,
        };
        let device_name = device.name().unwrap_or_default();
        let default_name =
            (opened_on_default && !device_name.is_empty()).then(|| device_name.clone());
        let health = Arc::new(StreamHealth::new(Instant::now()));

        let thread_device = device.clone();
        let level_cb = self.level_cb.clone();
        let selected_channel = self.selected_channel;
        let config_cache = Arc::clone(&self.config_cache);
        let stream_health = Arc::clone(&health);

        let worker = std::thread::spawn(move || {
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_flag_for_stream = stop_flag.clone();
            let init_result = (|| -> Result<(cpal::Stream, u32), String> {
                let config_started = Instant::now();
                let cached_config = config_cache
                    .lock()
                    .unwrap()
                    .as_ref()
                    .filter(|(name, _)| !device_name.is_empty() && *name == device_name)
                    .map(|(_, cfg)| cfg.clone());
                let config_was_cached = cached_config.is_some();
                let config = match cached_config {
                    Some(cfg) => cfg,
                    None => AudioRecorder::get_preferred_config(&thread_device)
                        .map_err(|e| format!("Failed to fetch preferred config: {e}"))?,
                };
                let config_elapsed = config_started.elapsed();

                let sample_rate = config.sample_rate().0;
                let channels = config.channels() as usize;

                log::info!(
                    "Using device: {:?}\nSample rate: {}\nChannels: {}\nFormat: {:?}",
                    thread_device.name(),
                    sample_rate,
                    channels,
                    config.sample_format()
                );

                if let Some(channel) = selected_channel {
                    if channel < channels {
                        log::info!("Using selected input channel: {}", channel + 1);
                    } else {
                        log::warn!(
                            "Selected input channel {} is out of range for a {}-channel device; averaging all channels instead",
                            channel + 1,
                            channels
                        );
                    }
                } else {
                    log::info!("Averaging all {} input channels", channels);
                }

                let build_started = Instant::now();
                let stream = match config.sample_format() {
                    cpal::SampleFormat::U8 => AudioRecorder::build_stream::<u8>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_health),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I8 => AudioRecorder::build_stream::<i8>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_health),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I16 => AudioRecorder::build_stream::<i16>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_health),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I32 => AudioRecorder::build_stream::<i32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_health),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::F32 => AudioRecorder::build_stream::<f32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_health),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    sample_format => {
                        return Err(format!("Unsupported sample format: {sample_format:?}"));
                    }
                };
                let build_elapsed = build_started.elapsed();

                let play_started = Instant::now();
                stream
                    .play()
                    .map_err(|e| format!("Failed to start microphone stream: {e}"))?;
                log::debug!(
                    "mic worker init: fetch_config={:?} (cached={}) build_stream={:?} play={:?}",
                    config_elapsed,
                    config_was_cached,
                    build_elapsed,
                    play_started.elapsed()
                );

                // The device accepted this config; remember it so the next
                // open skips the HAL property queries entirely.
                if !config_was_cached && !device_name.is_empty() {
                    *config_cache.lock().unwrap() = Some((device_name, config));
                }

                Ok((stream, sample_rate))
            })();

            match init_result {
                Ok((stream, sample_rate)) => {
                    let _ = init_tx.send(Ok(()));
                    // Timestamp for the play()-returned -> first-samples gap the
                    // init handshake can't see (hardware dependent).
                    let stream_running_at = Instant::now();
                    // Keep the stream alive while we process samples.
                    run_consumer(
                        sample_rate,
                        sample_rx,
                        cmd_rx,
                        level_cb,
                        stop_flag,
                        stream_running_at,
                    );
                    drop(stream);
                }
                Err(error_message) => {
                    // A failed open may mean the cached config went stale
                    // (device re-plugged, rate/format changed in the OS).
                    // Drop it so the next attempt re-queries the device.
                    *config_cache.lock().unwrap() = None;
                    log::error!("{error_message}");
                    let _ = init_tx.send(Err(error_message));
                }
            }
        });

        match init_rx.recv() {
            Ok(Ok(())) => {
                // The setup is not a stall: wait for a first callback from here.
                health.mark(Instant::now());
                self.device = Some(device);
                self.cmd_tx = Some(cmd_tx);
                self.worker_handle = Some(worker);
                self.health = Some(health);
                self.default_name = default_name;
                Ok(())
            }
            Ok(Err(error_message)) => {
                let _ = worker.join();
                let kind = if is_microphone_access_denied(&error_message) {
                    std::io::ErrorKind::PermissionDenied
                } else {
                    std::io::ErrorKind::Other
                };
                Err(Box::new(Error::new(kind, error_message)))
            }
            Err(recv_error) => {
                let _ = worker.join();
                Err(Box::new(Error::other(format!(
                    "Failed to initialize microphone worker: {recv_error}"
                ))))
            }
        }
    }

    /// Queue a recording start and return a one-shot receiver that resolves only
    /// after the first real microphone sample chunk has entered the capture path.
    /// `Stream::play()` returning is not sufficient: some Bluetooth and USB
    /// devices take much longer to begin delivering callbacks.
    pub fn start(&self) -> Result<mpsc::Receiver<()>, Box<dyn std::error::Error>> {
        let tx = self
            .cmd_tx
            .as_ref()
            .ok_or_else(|| Error::other("Recorder is not open"))?;
        let (ready_tx, ready_rx) = mpsc::channel();
        tx.send(Cmd::Start(Instant::now(), ready_tx))?;
        Ok(ready_rx)
    }

    /// A device cpal reported gone before its first chunk sent nothing the
    /// consumer could miss, so its report counts too.
    pub fn stop(&self) -> Result<Capture, Box<dyn std::error::Error>> {
        self.stop_with_tail(None)
    }

    /// Like `stop`, but keeps recording for the release tail first.
    pub fn stop_with_tail(
        &self,
        tail: Option<ReleaseTailRequest>,
    ) -> Result<Capture, Box<dyn std::error::Error>> {
        let (resp_tx, resp_rx) = mpsc::channel();
        if let Some(tx) = &self.cmd_tx {
            tx.send(Cmd::Stop(resp_tx, tail))?;
        }
        let mut capture = resp_rx.recv()?;
        capture.mic_lost |= self
            .health
            .as_ref()
            .is_some_and(|health| health.failed.load(Ordering::Relaxed));
        self.note_clip(&capture);
        Ok(capture)
    }

    /// A clip of exact zeros, from first sample to last, is not a quiet
    /// room: no microphone's noise is that clean. It is what a stream that
    /// outlived its device's owner delivers, so ask for a rebuild before the
    /// next recording rather than discard every one until a restart.
    fn note_clip(&self, capture: &Capture) {
        self.note_clip_at(capture, Instant::now());
    }

    fn note_clip_at(&self, capture: &Capture, now: Instant) {
        let Some(health) = &self.health else { return };
        if capture.samples.is_empty() {
            return;
        }
        let mut last = self.last_zero_reopen.lock().unwrap();
        if !is_digital_silence(&capture.samples) {
            *last = None;
            return;
        }
        if last.is_some_and(|at| now.duration_since(at) < ZERO_REOPEN_EVERY) {
            return;
        }
        log::warn!("The clip held only digital silence; the stream will be reopened");
        *last = Some(now);
        health.delivered_only_zeros.store(true, Ordering::Relaxed);
    }

    /// True when the active capture stream must be rebuilt.
    ///
    /// cpal may report a device disconnect asynchronously without closing its
    /// callback channel, so also honor the error callback's explicit flag. A
    /// stream on the system default gets no such report, so a stream is also
    /// rebuilt once its callbacks stop or another microphone is the default.
    pub fn needs_reopen(&self) -> bool {
        let Some(health) = &self.health else {
            return false;
        };
        if health.failed.load(Ordering::Relaxed)
            || health.delivered_only_zeros.load(Ordering::Relaxed)
            || self
                .worker_handle
                .as_ref()
                .is_some_and(|handle| handle.is_finished())
        {
            return true;
        }
        if health.stalled(Instant::now()) {
            log::warn!("The microphone stream made no callback for {STALL:?}");
            return true;
        }
        if self.left_default(|| cpal::default_host().default_input_device()?.name().ok()) {
            log::info!("Another microphone is now the system default");
            return true;
        }
        false
    }

    /// A stream opened on the system default stays on the device that was
    /// the default then. Reopened, it records from the current default, as a
    /// stream opened for each dictation does.
    fn left_default(&self, default_now: impl FnOnce() -> Option<String>) -> bool {
        self.default_name
            .as_deref()
            .is_some_and(|opened_on| default_now().as_deref() != Some(opened_on))
    }

    pub fn close(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(Cmd::Shutdown);
        }
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
        self.device = None;
        self.health = None;
        self.default_name = None;
        Ok(())
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        sample_tx: mpsc::Sender<AudioChunk>,
        channels: usize,
        selected_channel: Option<usize>,
        stop_flag: Arc<AtomicBool>,
        health: Arc<StreamHealth>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: Sample + SizedSample + Send + 'static,
        f32: cpal::FromSample<T>,
    {
        let mut output_buffer = Vec::new();
        let mut eos_sent = false;
        // Resolve the effective channel to use. If the selected channel is
        // out of range for this device, fall back to averaging all channels.
        let use_channel: Option<usize> = match selected_channel {
            Some(ch) if ch < channels => Some(ch),
            Some(_) => None,
            None => None,
        };

        let callback_health = Arc::clone(&health);
        let stream_cb = move |data: &[T], _: &cpal::InputCallbackInfo| {
            // Every call shows the device is there, stopped or not.
            callback_health.mark(Instant::now());
            if stop_flag.load(Ordering::Relaxed) {
                if !eos_sent {
                    let _ = sample_tx.send(AudioChunk::EndOfStream);
                    eos_sent = true;
                }
                return;
            }
            eos_sent = false;

            output_buffer.clear();

            if channels == 1 {
                output_buffer.extend(data.iter().map(|&sample| sample.to_sample::<f32>()));
            } else {
                let frame_count = data.len() / channels;
                output_buffer.reserve(frame_count);

                if let Some(ch) = use_channel {
                    for frame in data.chunks_exact(channels) {
                        let mono_sample = frame[ch].to_sample::<f32>();
                        output_buffer.push(mono_sample);
                    }
                } else {
                    for frame in data.chunks_exact(channels) {
                        let mono_sample = frame
                            .iter()
                            .map(|&sample| sample.to_sample::<f32>())
                            .sum::<f32>()
                            / channels as f32;
                        output_buffer.push(mono_sample);
                    }
                }
            }

            if sample_tx
                .send(AudioChunk::Samples(output_buffer.clone()))
                .is_err()
            {
                log::error!("Failed to send samples");
            }
        };

        device.build_input_stream(
            &config.clone().into(),
            stream_cb,
            move |err| {
                log::error!("Stream error: {}", err);
                health.failed.store(true, Ordering::Relaxed);
            },
            None,
        )
    }

    pub fn preferred_input_channel_count(
        device: &cpal::Device,
    ) -> Result<u16, Box<dyn std::error::Error>> {
        Ok(Self::get_preferred_config(device)?.channels())
    }

    fn get_preferred_config(
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, Box<dyn std::error::Error>> {
        // Use the device's native/default sample rate and let the FrameResampler
        // in run_consumer() downsample to 16kHz. This avoids forcing hardware into
        // a non-native rate which can cause issues on some devices (Bluetooth
        // codecs, certain ALSA drivers, etc.).
        let default_config = device.default_input_config()?;
        let target_rate = default_config.sample_rate();

        let supported_configs = match device.supported_input_configs() {
            Ok(configs) => configs,
            Err(e) => {
                log::warn!("Could not enumerate input configs ({e}), using device default");
                return Ok(default_config);
            }
        };
        let mut best_config: Option<cpal::SupportedStreamConfigRange> = None;

        for config_range in supported_configs {
            if config_range.min_sample_rate() <= target_rate
                && config_range.max_sample_rate() >= target_rate
            {
                match best_config {
                    None => best_config = Some(config_range),
                    Some(ref current) => {
                        let score = |fmt: cpal::SampleFormat| match fmt {
                            cpal::SampleFormat::F32 => 4,
                            cpal::SampleFormat::I16 => 3,
                            cpal::SampleFormat::I32 => 2,
                            _ => 1,
                        };

                        if score(config_range.sample_format()) > score(current.sample_format()) {
                            best_config = Some(config_range);
                        }
                    }
                }
            }
        }

        if let Some(config) = best_config {
            return Ok(config.with_sample_rate(target_rate));
        }

        log::warn!(
            "No supported config matched device default rate {:?}, using default config",
            target_rate
        );
        Ok(default_config)
    }
}

/// The least time between two reopens asked for by zero clips.
const ZERO_REOPEN_EVERY: Duration = Duration::from_secs(30);

/// A clip with samples that are all exactly 0.0.
fn is_digital_silence(samples: &[f32]) -> bool {
    !samples.is_empty() && samples.iter().all(|sample| *sample == 0.0)
}

pub fn is_microphone_access_denied(error_message: &str) -> bool {
    let normalized = error_message.to_lowercase();
    normalized.contains("access is denied") || normalized.contains("permission denied")
}

pub fn is_no_input_device_error(error_message: &str) -> bool {
    let normalized = error_message.to_lowercase();
    normalized.contains("no input device found")
        || (normalized.contains("failed to fetch preferred config")
            && normalized.contains("coreaudio"))
}

fn append_recording_frame_with_limit(
    samples: &[f32],
    recording: bool,
    out_buf: &mut Vec<f32>,
    limit: usize,
) -> bool {
    if !recording {
        return false;
    }
    let remaining = limit.saturating_sub(out_buf.len());
    out_buf.extend_from_slice(&samples[..samples.len().min(remaining)]);
    out_buf.len() >= limit
}

fn append_recording_frame(samples: &[f32], recording: bool, out_buf: &mut Vec<f32>) -> bool {
    append_recording_frame_with_limit(
        samples,
        recording,
        out_buf,
        constants::MAX_RECORDING_SAMPLES,
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests;

/// The next queued command. Windows keeps the command it waited for after its
/// data callback closed; macOS only polls the queue.
#[cfg(target_os = "windows")]
fn next_command(held: &mut Option<Cmd>, cmd_rx: &mpsc::Receiver<Cmd>) -> Option<Cmd> {
    held.take().or_else(|| cmd_rx.try_recv().ok())
}

#[cfg(not(target_os = "windows"))]
fn next_command(_held: &mut Option<Cmd>, cmd_rx: &mpsc::Receiver<Cmd>) -> Option<Cmd> {
    cmd_rx.try_recv().ok()
}

/// No chunk for this long during the tail means the device stopped
/// delivering: the stop's own drain finds that out, so the tail gives up.
const TAIL_STALL: Duration = Duration::from_millis(250);

/// Keeps capturing after the release until `release_tail` says the user is
/// done, the request is aborted, or the stream stalls. The clip so far sets
/// the speech threshold; nothing is recorded as a tail for a tap.
fn record_release_tail(
    request: &ReleaseTailRequest,
    sample_rx: &mpsc::Receiver<AudioChunk>,
    frame_resampler: &mut FrameResampler,
    processed_samples: &mut Vec<f32>,
    capture_limit_logged: &mut bool,
) {
    let Some(threshold) = release_tail::threshold_for(processed_samples) else {
        return;
    };
    let ended_quiet = release_tail::ended_quiet(processed_samples, threshold);
    let mut tail = ReleaseTail::new(request.config, threshold, ended_quiet);
    let started = Instant::now();
    // The tail is measured in audio; this bounds it in time too.
    let deadline = started + request.config.cap() + TAIL_STALL;
    let mut last_chunk = started;
    let start_len = processed_samples.len();
    let mut judged = start_len;

    'tail: while Instant::now() < deadline && !(request.abort)() {
        match sample_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(AudioChunk::Samples(raw)) => {
                last_chunk = Instant::now();
                frame_resampler.push(&raw, &mut |frame: &[f32]| {
                    if append_recording_frame(frame, true, processed_samples)
                        && !*capture_limit_logged
                    {
                        log::warn!("Recording reached the 15-minute sample limit");
                        *capture_limit_logged = true;
                    }
                });
                while processed_samples.len() >= judged + FRAME {
                    let level = frame_levels(&processed_samples[judged..judged + FRAME])[0];
                    judged += FRAME;
                    if tail.push_frame(level) == Step::Finish {
                        break 'tail;
                    }
                }
                if *capture_limit_logged {
                    break;
                }
            }
            Ok(AudioChunk::EndOfStream) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_chunk.elapsed() > TAIL_STALL {
                    break;
                }
            }
        }
    }
    log::debug!(
        "Release tail recorded {:.0}ms (recording ended {})",
        (processed_samples.len() - start_len) as f64 * 1000.0
            / f64::from(constants::TARGET_SAMPLE_RATE),
        if ended_quiet { "quiet" } else { "in speech" }
    );
}

#[allow(clippy::too_many_arguments)]
fn run_consumer(
    in_sample_rate: u32,
    sample_rx: mpsc::Receiver<AudioChunk>,
    cmd_rx: mpsc::Receiver<Cmd>,
    level_cb: Option<Arc<dyn Fn(Vec<f32>) + Send + Sync + 'static>>,
    stop_flag: Arc<AtomicBool>,
    stream_running_at: Instant,
) {
    let mut frame_resampler = FrameResampler::new(
        in_sample_rate as usize,
        constants::TARGET_SAMPLE_RATE as usize,
        Duration::from_millis(30),
    );

    let mut processed_samples = Vec::<f32>::new();
    let mut recording = false;
    let mut capture_limit_logged = false;

    // First-chunk arrival exposes the play()->samples-flowing gap; the
    // first-captured log confirms capture begins with the chunk in flight
    // when Cmd::Start lands. Once the stream has delivered, a stop that gets
    // no EndOfStream back means its callbacks stopped: the device is gone.
    let mut first_chunk_arrived = false;
    let mut awaiting_first_captured_chunk: Option<Instant> = None;
    let mut capture_ready_tx: Option<mpsc::Sender<()>> = None;

    const BUCKETS: usize = 16;
    // Scale the FFT window to the device sample rate so the analysis window
    // (~33 ms) and frequency resolution (~30 Hz/bin) stay roughly constant
    // across devices. A fixed 512-sample window collapses the low vocal
    // buckets onto a single bin at 48 kHz (e.g. built-in laptop mics), and
    // would stutter at ~4-8 updates/sec on an 8-16 kHz Bluetooth headset.
    // Targets: 48 kHz -> 2048, 16 kHz -> 512, 8 kHz -> 256.
    let target_window = (f64::from(in_sample_rate) / 30.0).round() as usize;
    let window_size = [256usize, 512, 1024, 2048]
        .into_iter()
        .min_by_key(|w| w.abs_diff(target_window))
        .unwrap();
    let mut visualizer = AudioVisualiser::new(
        in_sample_rate,
        window_size,
        BUCKETS,
        400.0,  // vocal_min_hz
        4000.0, // vocal_max_hz
    );

    // Poll commands even when a disconnected device stops producing samples
    // without closing its CoreAudio stream.
    loop {
        let mut disconnected_command: Option<Cmd> = None;
        let mut pending = match sample_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => Some(chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            #[cfg(not(target_os = "windows"))]
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            #[cfg(target_os = "windows")]
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // WASAPI drops its data callback when the stream thread exits
                // on an error. Keep the captured audio until Stop asks for it.
                // Block on commands instead of spinning on the closed channel.
                disconnected_command = cmd_rx.recv().ok();
                if disconnected_command.is_none() {
                    break;
                }
                None
            }
        };

        // Handle pending commands BEFORE the in-flight chunk so a Start
        // captures it. Polling after processing would silently drop one buffer
        // period of audio (~10ms built-in, up to ~100ms on Bluetooth) at every
        // recording start.
        while let Some(cmd) = next_command(&mut disconnected_command, &cmd_rx) {
            match cmd {
                Cmd::Start(sent_at, ready_tx) => {
                    log::debug!(
                        "Cmd::Start processed {:?} after send; capture begins with {} chunk",
                        sent_at.elapsed(),
                        if pending.is_some() {
                            "the in-flight"
                        } else {
                            "the next available"
                        }
                    );
                    awaiting_first_captured_chunk = Some(Instant::now());
                    capture_ready_tx = Some(ready_tx);
                    stop_flag.store(false, Ordering::Relaxed);
                    processed_samples.clear();
                    capture_limit_logged = false;
                    recording = true;
                    visualizer.reset();
                    frame_resampler.reset();
                }
                Cmd::Stop(reply_tx, tail) => {
                    recording = false;
                    // If Stop was queued before the first chunk, dropping this
                    // sender prevents a stale ready UI event or start chime.
                    capture_ready_tx = None;
                    awaiting_first_captured_chunk = None;

                    // The chunk in hand arrived before the stop; it belongs to
                    // the recording, so feed it ahead of the drain below.
                    if let Some(AudioChunk::Samples(raw)) =
                        pending.take().filter(|_| !capture_limit_logged)
                    {
                        first_chunk_arrived = true;
                        frame_resampler.push(&raw, &mut |frame: &[f32]| {
                            if append_recording_frame(frame, true, &mut processed_samples)
                                && !capture_limit_logged
                            {
                                log::warn!("Recording reached the 15-minute sample limit");
                                capture_limit_logged = true;
                            }
                        });
                    }

                    if let Some(request) = tail.filter(|_| !capture_limit_logged) {
                        record_release_tail(
                            &request,
                            &sample_rx,
                            &mut frame_resampler,
                            &mut processed_samples,
                            &mut capture_limit_logged,
                        );
                    }
                    stop_flag.store(true, Ordering::Relaxed);

                    // Drain all remaining audio until the producer confirms end-of-stream.
                    // The cpal callback sees the stop flag, sends EndOfStream, and goes
                    // silent — guaranteeing every captured sample is in the channel
                    // ahead of the sentinel.
                    let mut mic_lost = false;
                    loop {
                        match sample_rx.recv_timeout(Duration::from_secs(2)) {
                            Ok(AudioChunk::Samples(remaining)) => {
                                first_chunk_arrived = true;
                                if !capture_limit_logged {
                                    frame_resampler.push(&remaining, &mut |frame: &[f32]| {
                                        if append_recording_frame(
                                            frame,
                                            true,
                                            &mut processed_samples,
                                        ) && !capture_limit_logged
                                        {
                                            log::warn!(
                                                "Recording reached the 15-minute sample limit"
                                            );
                                            capture_limit_logged = true;
                                        }
                                    });
                                }
                            }
                            Ok(AudioChunk::EndOfStream) => break,
                            Err(_) => {
                                log::warn!("Timed out waiting for EndOfStream from audio callback");
                                mic_lost = first_chunk_arrived;
                                break;
                            }
                        }
                    }

                    if !capture_limit_logged {
                        frame_resampler.finish(&mut |frame: &[f32]| {
                            if append_recording_frame(frame, true, &mut processed_samples)
                                && !capture_limit_logged
                            {
                                log::warn!("Recording reached the 15-minute sample limit");
                                capture_limit_logged = true;
                            }
                        });
                    }

                    let _ = reply_tx.send(Capture {
                        samples: std::mem::take(&mut processed_samples),
                        mic_lost,
                    });

                    // Resume the audio callback so the consumer loop can continue
                    // receiving chunks (important for always-on microphone mode).
                    stop_flag.store(false, Ordering::Relaxed);
                }
                Cmd::Shutdown => {
                    stop_flag.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }

        let raw = match pending.take() {
            Some(AudioChunk::Samples(s)) => s,
            // EndOfStream, or the chunk was consumed by a Stop above.
            _ => continue,
        };

        let chunk_ms = raw.len() as f64 * 1000.0 / in_sample_rate as f64;
        if !first_chunk_arrived {
            first_chunk_arrived = true;
            log::debug!(
                "first audio chunk arrived {:?} after stream start ({:.1}ms of audio)",
                stream_running_at.elapsed(),
                chunk_ms
            );
        }

        // In always-on mode the capture stream stays open continuously for
        // zero-latency start, so while idle (not recording) there is nothing to
        // do with a chunk. Skip both the level-meter FFT and resampler while
        // idle, and after the hard sample cap, to avoid work whose output would
        // be discarded. Both are reset on Cmd::Start.
        if recording && !capture_limit_logged {
            if let Some(buckets) = visualizer.feed(&raw) {
                if let Some(cb) = &level_cb {
                    cb(buckets);
                }
            }

            frame_resampler.push(&raw, &mut |frame: &[f32]| {
                if append_recording_frame(frame, recording, &mut processed_samples)
                    && !capture_limit_logged
                {
                    log::warn!("Recording reached the 15-minute sample limit");
                    capture_limit_logged = true;
                }
            });
        }

        if recording {
            if let Some(started) = awaiting_first_captured_chunk.take() {
                log::debug!(
                    "first captured chunk ({:.1}ms of audio) processed {:?} after Cmd::Start",
                    chunk_ms,
                    started.elapsed()
                );
            }
            if let Some(ready_tx) = capture_ready_tx.take() {
                let _ = ready_tx.send(());
            }
        }
    }
}
