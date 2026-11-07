use crate::audio_toolkit::AudioRecorder;
use crate::settings::get_settings;
use log::error;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

mod microphone;
mod mute;
mod recording;

pub use microphone::preferred_microphone;
use microphone::*;
use mute::*;

#[derive(Clone, Debug)]
pub enum RecordingState {
    Idle,
    Recording { binding_id: String },
    Stopping,
}

#[derive(Clone, Debug)]
pub enum MicrophoneMode {
    AlwaysOn,
    OnDemand,
}

/// One recording session's first-sample notification. Waiting on this never
/// blocks the shortcut coordinator: callers hand it to a dedicated worker.
pub struct RecordingReadiness {
    receiver: mpsc::Receiver<()>,
    generation: u64,
}

impl RecordingReadiness {
    pub fn wait(self) -> bool {
        self.receiver.recv().is_ok()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone)]
pub struct AudioRecordingManager {
    /// Never assign through this directly — route every write through
    /// `set_state()`, which keeps `recording_active` in sync.
    state: Arc<Mutex<RecordingState>>,
    mode: Arc<Mutex<MicrophoneMode>>,
    app_handle: tauri::AppHandle,

    recorder: Arc<Mutex<Option<AudioRecorder>>>,
    is_open: Arc<Mutex<bool>>,
    is_recording: Arc<Mutex<bool>>,
    mute_state: Arc<Mutex<MuteState>>,
    close_generation: Arc<AtomicU64>,
    cancel_generation: Arc<AtomicU64>,
    recording_active: Arc<AtomicBool>,
    /// True only while a recorder is accepting samples. `recording_active`
    /// intentionally remains true through the trailing-buffer `Stopping`
    /// state; callers that just attempted a new start need this narrower
    /// signal so they cannot mistake an old stop for a new recording.
    recording_started: Arc<AtomicBool>,
    /// Invalidates asynchronous first-sample UI/chime work when a recording is
    /// stopped or cancelled. This prevents a slow device from producing a late
    /// "ready" indication for a session the user already ended.
    capture_generation: Arc<AtomicU64>,
    /// Resolution of a *named* microphone (selected or clamshell) to its cpal
    /// device, cached so on-demand recording starts skip the full device
    /// enumeration (~40-110ms). Keyed by the resolved name, so a settings
    /// change misses naturally; cleared when an open fails (device unplugged)
    /// so the retry re-enumerates. The system-default case is never cached —
    /// the recorder resolves the current default itself, cheaply.
    cached_device: Arc<Mutex<Option<(String, cpal::Device)>>>,
}

impl AudioRecordingManager {
    /// Builds the manager. If the always-on microphone cannot open (unplugged,
    /// busy or denied at login), the manager falls back to on-demand capture
    /// for this session and returns the error so the caller can tell the user.
    /// The persisted preference is left as is, so the next launch tries again.
    pub fn new(app: &tauri::AppHandle) -> (Self, Option<anyhow::Error>) {
        let settings = get_settings(app);
        let mode = if settings.always_on_microphone {
            MicrophoneMode::AlwaysOn
        } else {
            MicrophoneMode::OnDemand
        };

        let manager = Self {
            state: Arc::new(Mutex::new(RecordingState::Idle)),
            mode: Arc::new(Mutex::new(mode.clone())),
            app_handle: app.clone(),

            recorder: Arc::new(Mutex::new(None)),
            is_open: Arc::new(Mutex::new(false)),
            is_recording: Arc::new(Mutex::new(false)),
            mute_state: Arc::new(Mutex::new(MuteState::default())),
            close_generation: Arc::new(AtomicU64::new(0)),
            cancel_generation: Arc::new(AtomicU64::new(0)),
            recording_active: Arc::new(AtomicBool::new(false)),
            recording_started: Arc::new(AtomicBool::new(false)),
            capture_generation: Arc::new(AtomicU64::new(0)),
            cached_device: Arc::new(Mutex::new(None)),
        };

        // The persisted AlwaysOn value was committed by a previous successful
        // transition, so startup opens the stream without writing or emitting
        // it again.
        let mut startup_error = None;
        if matches!(mode, MicrophoneMode::AlwaysOn) {
            if let Err(error) = manager.start_microphone_stream() {
                error!("Always-on microphone failed to open at startup: {error:#}");
                if let Ok(mut mode) = manager.mode.lock() {
                    *mode = MicrophoneMode::OnDemand;
                }
                startup_error = Some(error);
            }
        }

        (manager, startup_error)
    }

    pub fn invalidate_device_cache(&self) {
        *self.cached_device.lock().unwrap() = None;
    }

    /// The one place `state` is written. Derives `recording_active` (the
    /// lock-free mirror read by `is_recording()`) from the new value itself,
    /// so the two can never drift: a new `RecordingState` variant only needs
    /// its active-set membership decided here, once.
    fn set_state(&self, guard: &mut RecordingState, new_state: RecordingState) {
        *guard = new_state;
        self.recording_active.store(
            matches!(
                *guard,
                RecordingState::Recording { .. } | RecordingState::Stopping
            ),
            Ordering::SeqCst,
        );
        self.recording_started.store(
            matches!(*guard, RecordingState::Recording { .. }),
            Ordering::SeqCst,
        );
    }

    /// Invalidate pending first-sample UI and audio-feedback work immediately.
    /// Called at the beginning of stop, before the slower capture drain starts.
    pub fn invalidate_recording_readiness(&self) {
        self.capture_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn is_recording_readiness_current(&self, generation: u64) -> bool {
        self.capture_generation.load(Ordering::Acquire) == generation
    }

    pub fn cancel_generation(&self) -> u64 {
        self.cancel_generation.load(Ordering::Acquire)
    }

    pub fn was_cancelled_since(&self, generation: u64) -> bool {
        self.cancel_generation.load(Ordering::Acquire) != generation
    }

    pub fn is_recording(&self) -> bool {
        // Lock-free: mirrors the `state` {Recording, Stopping} membership via
        // an atomic maintained by `set_state()`. Polled from the webview/main
        // thread, so it MUST NOT take the `state` mutex (a worker can hold it
        // across a slow CoreAudio open/close → main-thread deadlock / UI
        // freeze).
        self.recording_active.load(Ordering::SeqCst)
    }

    /// Returns true only after this manager has admitted an active recorder.
    /// It is safe for the coordinator to use after `try_start_recording`; a
    /// `Stopping` recorder belongs to an earlier operation and is not a
    /// successful new start.
    pub fn is_recording_started(&self) -> bool {
        self.recording_started.load(Ordering::SeqCst)
    }

    /// A prior stop still owns the recorder. The coordinator retries a start
    /// after this becomes false instead of treating it as an already-started
    /// recording.
    pub fn is_stopping(&self) -> bool {
        self.is_recording() && !self.is_recording_started()
    }
}

/// Decides whether a stop worker still owns the currently active recording.
/// Callers must hold the recording-state mutex while invoking this predicate.
fn can_claim_stop(
    state: &RecordingState,
    binding_id: &str,
    expected_cancel_generation: u64,
    current_cancel_generation: u64,
) -> bool {
    expected_cancel_generation == current_cancel_generation
        && matches!(state, RecordingState::Recording { binding_id: active } if active == binding_id)
}

#[cfg(test)]
mod tests;
