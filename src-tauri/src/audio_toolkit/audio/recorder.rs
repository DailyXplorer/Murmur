use std::{
    io::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
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
};

enum Cmd {
    Start(Instant, mpsc::Sender<()>),
    Stop(mpsc::Sender<Vec<f32>>),
    Shutdown,
}

enum AudioChunk {
    Samples(Vec<f32>),
    EndOfStream,
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
    /// Set by cpal when the active input stream can no longer capture.
    stream_error: Arc<AtomicBool>,
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
            stream_error: Arc::new(AtomicBool::new(false)),
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
                return Ok(()); // already open
            }
            log::warn!("Capture stream failed; rebuilding microphone stream");
            let _ = self.close();
        }

        self.stream_error.store(false, Ordering::Relaxed);

        let (sample_tx, sample_rx) = mpsc::channel::<AudioChunk>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let host = cpal::default_host();
        let device = match device {
            Some(dev) => dev,
            None => host
                .default_input_device()
                .ok_or_else(|| Error::new(std::io::ErrorKind::NotFound, "No input device found"))?,
        };

        let thread_device = device.clone();
        let level_cb = self.level_cb.clone();
        let selected_channel = self.selected_channel;
        let config_cache = Arc::clone(&self.config_cache);
        let stream_error = Arc::clone(&self.stream_error);

        let worker = std::thread::spawn(move || {
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_flag_for_stream = stop_flag.clone();
            let init_result = (|| -> Result<(cpal::Stream, u32), String> {
                let config_started = Instant::now();
                let device_name = thread_device.name().unwrap_or_default();
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
                        Arc::clone(&stream_error),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I8 => AudioRecorder::build_stream::<i8>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_error),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I16 => AudioRecorder::build_stream::<i16>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_error),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I32 => AudioRecorder::build_stream::<i32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_error),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::F32 => AudioRecorder::build_stream::<f32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        selected_channel,
                        stop_flag_for_stream,
                        Arc::clone(&stream_error),
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
                self.device = Some(device);
                self.cmd_tx = Some(cmd_tx);
                self.worker_handle = Some(worker);
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

    pub fn stop(&self) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let (resp_tx, resp_rx) = mpsc::channel();
        if let Some(tx) = &self.cmd_tx {
            tx.send(Cmd::Stop(resp_tx))?;
        }
        Ok(resp_rx.recv()?) // wait for the samples
    }

    /// True when the active capture stream must be rebuilt.
    ///
    /// cpal may report a device disconnect asynchronously without closing its
    /// callback channel, so also honor the error callback's explicit flag.
    pub fn needs_reopen(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
            || self
                .worker_handle
                .as_ref()
                .is_some_and(|handle| handle.is_finished())
    }

    pub fn close(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(Cmd::Shutdown);
        }
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
        self.device = None;
        Ok(())
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        sample_tx: mpsc::Sender<AudioChunk>,
        channels: usize,
        selected_channel: Option<usize>,
        stop_flag: Arc<AtomicBool>,
        stream_error: Arc<AtomicBool>,
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
            Some(_) => None, // out of range, fall back to average
            None => None,    // user chose "average all"
        };

        let stream_cb = move |data: &[T], _: &cpal::InputCallbackInfo| {
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
                stream_error.store(true, Ordering::Relaxed);
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

        // Try to find the best sample format at the device's default rate
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
                        // Prioritize F32 > I16 > I32 > others
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

        // Fall back to device default if no config matched (exotic/virtual devices)
        log::warn!(
            "No supported config matched device default rate {:?}, using default config",
            target_rate
        );
        Ok(default_config)
    }
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
mod tests {
    use super::{
        append_recording_frame_with_limit, is_microphone_access_denied, is_no_input_device_error,
        run_consumer, AudioRecorder, Cmd,
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn unopened_recorder_does_not_need_reopen() {
        // No worker has been spawned yet, so there is nothing to reap. Guards
        // against inverting the "no worker" case, which would make every first
        // open() take the rebuild path.
        let recorder = AudioRecorder::new().expect("recorder");
        assert!(!recorder.needs_reopen());
    }

    #[test]
    fn stream_error_requires_reopen() {
        let recorder = AudioRecorder::new().expect("recorder");
        recorder.stream_error.store(true, Ordering::Relaxed);
        assert!(recorder.needs_reopen());
    }

    #[test]
    fn shutdown_is_processed_without_audio_samples() {
        let (sample_tx, sample_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_consumer(
                48_000,
                sample_rx,
                cmd_rx,
                None,
                Arc::new(AtomicBool::new(false)),
                Instant::now(),
            );
            let _ = done_tx.send(());
        });

        cmd_tx.send(Cmd::Shutdown).expect("send shutdown");
        let stopped = done_rx.recv_timeout(Duration::from_secs(1));

        // Unblock the old implementation so a failing test still exits cleanly.
        drop(sample_tx);
        worker.join().expect("join consumer");
        assert!(stopped.is_ok(), "shutdown waited for an audio sample");
    }

    #[test]
    fn detects_access_is_denied() {
        assert!(is_microphone_access_denied("Access is denied"));
    }

    #[test]
    fn detects_permission_denied() {
        assert!(is_microphone_access_denied("permission denied"));
    }

    #[test]
    fn does_not_match_unrelated_errors() {
        assert!(!is_microphone_access_denied("device not found"));
    }

    #[test]
    fn detects_no_input_device() {
        assert!(is_no_input_device_error("No input device found"));
    }

    #[test]
    fn detects_coreaudio_config_error() {
        assert!(is_no_input_device_error(
            "Failed to fetch preferred config: A backend-specific error has occurred: An unknown error unknown to the coreaudio-rs API occurred"
        ));
    }

    #[test]
    fn does_not_match_other_errors_for_no_device() {
        assert!(!is_no_input_device_error("permission denied"));
        assert!(!is_no_input_device_error("device not found"));
    }

    #[test]
    fn recording_frames_never_exceed_the_sample_limit() {
        let mut samples = vec![1.0, 2.0];
        assert!(append_recording_frame_with_limit(
            &[3.0, 4.0],
            true,
            &mut samples,
            3
        ));
        assert_eq!(samples, vec![1.0, 2.0, 3.0]);

        assert!(append_recording_frame_with_limit(
            &[5.0],
            true,
            &mut samples,
            3
        ));
        assert_eq!(samples, vec![1.0, 2.0, 3.0]);
    }
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

    // ---------- latency instrumentation ---------------------------------- //
    // First-chunk arrival exposes the play()->samples-flowing gap; the
    // first-captured log confirms capture begins with the chunk in flight
    // when Cmd::Start lands.
    let mut first_chunk_logged = false;
    let mut awaiting_first_captured_chunk: Option<Instant> = None;
    let mut capture_ready_tx: Option<mpsc::Sender<()>> = None;

    // ---------- spectrum visualisation setup ---------------------------- //
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
        let mut pending = match sample_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => Some(chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Handle pending commands BEFORE the in-flight chunk so a Start
        // captures it. Commands used to be polled after processing, which
        // silently dropped one buffer period of audio (~10ms built-in, up to
        // ~100ms on Bluetooth) at every recording start.
        while let Ok(cmd) = cmd_rx.try_recv() {
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
                Cmd::Stop(reply_tx) => {
                    recording = false;
                    // If Stop was queued before the first chunk, dropping this
                    // sender prevents a stale ready UI event or start chime.
                    capture_ready_tx = None;
                    awaiting_first_captured_chunk = None;
                    stop_flag.store(true, Ordering::Relaxed);

                    // The chunk in hand arrived before the stop; it belongs to
                    // the recording, so feed it ahead of the drain below.
                    if let Some(AudioChunk::Samples(raw)) =
                        pending.take().filter(|_| !capture_limit_logged)
                    {
                        frame_resampler.push(&raw, &mut |frame: &[f32]| {
                            if append_recording_frame(frame, true, &mut processed_samples)
                                && !capture_limit_logged
                            {
                                log::warn!("Recording reached the 15-minute sample limit");
                                capture_limit_logged = true;
                            }
                        });
                    }

                    // Drain all remaining audio until the producer confirms end-of-stream.
                    // The cpal callback sees the stop flag, sends EndOfStream, and goes
                    // silent — guaranteeing every captured sample is in the channel
                    // ahead of the sentinel.
                    loop {
                        match sample_rx.recv_timeout(Duration::from_secs(2)) {
                            Ok(AudioChunk::Samples(remaining)) => {
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

                    let _ = reply_tx.send(std::mem::take(&mut processed_samples));

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
        if !first_chunk_logged {
            first_chunk_logged = true;
            log::debug!(
                "first audio chunk arrived {:?} after stream start ({:.1}ms of audio)",
                stream_running_at.elapsed(),
                chunk_ms
            );
        }

        // ---------- recording-time processing ---------------------------- //
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
