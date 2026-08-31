use crate::audio_toolkit::{list_input_devices, AudioRecorder};
use crate::helpers::clamshell;
use crate::settings::{get_settings, write_settings, AppSettings};
use crate::utils;
use log::{debug, error, info, trace, warn};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn set_mute(mute: bool) {
    use std::process::Command;
    let script = format!(
        "set volume output muted {}",
        if mute { "true" } else { "false" }
    );
    let _ = Command::new("osascript").args(["-e", &script]).output();
}

/// Reads the current system output mute state, mirroring `set_mute`.
///
/// Returns `Some(true)`/`Some(false)` when the state could be determined, or
/// `None` when `osascript` is missing or returns an error. Callers treat `None`
/// as "unknown" and fall back to unmuting on stop,
/// so we never strand the user's audio muted.
fn get_mute() -> Option<bool> {
    use std::process::Command;

    let out = Command::new("osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Restores the system mute state after our forced mute, given the state
/// captured just before we muted. We only ever need to unmute — and only when
/// the system was NOT already muted beforehand. If the prior state was muted,
/// we leave it muted (the user's own state). If it's unknown (`None`), we
/// default to unmuting so audio is never left stranded muted by us.
fn restore_mute(prev_muted: Option<bool>) {
    if prev_muted != Some(true) {
        set_mute(false);
    }
}

const TARGET_SAMPLE_RATE: usize = 16000;

/* ──────────────────────────────────────────────────────────────── */

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

/// Tracks our forced "mute while recording" so we can restore the user's audio
/// exactly as it was. `did_mute` is true while our mute is active; `prev_muted`
/// is the system mute state captured just before we muted, used to decide
/// whether to unmute on stop (so a system that was already muted stays muted).
#[derive(Debug, Default, Clone, Copy)]
struct MuteState {
    did_mute: bool,
    prev_muted: Option<bool>,
}

/// The persisted microphone preference currently in effect. Clamshell and
/// regular selections are kept distinct so losing a clamshell-only device does
/// not erase the user's normal microphone preference.
enum DesiredMicrophone {
    Default,
    Selected(String),
    Clamshell(String),
}

/// Result of resolving the persisted preference to a live cpal device.
/// `device: None` means cpal should open the system default. The unavailable
/// name is populated only when enumeration succeeded and confirmed that the
/// user's regular selected microphone is missing.
struct MicrophoneResolution {
    device: Option<cpal::Device>,
    unavailable_selected_microphone: Option<String>,
}

/* ──────────────────────────────────────────────────────────────── */

fn create_audio_recorder(
    app_handle: &tauri::AppHandle,
    selected_channel: Option<u16>,
) -> Result<AudioRecorder, anyhow::Error> {
    let recorder = AudioRecorder::new()
        .map_err(|e| anyhow::anyhow!("Failed to create AudioRecorder: {}", e))?
        .with_selected_channel(selected_channel)
        .with_level_callback({
            let app_handle = app_handle.clone();
            move |levels| {
                utils::emit_levels(&app_handle, &levels);
            }
        });

    Ok(recorder)
}

/// Outcome of rebuilding an idle stream after an input setting changes. The
/// caller must persist the setting that matches the returned live stream.
enum StreamRestartOutcome<T> {
    Updated(T),
    RestoredAfterUpdateFailure {
        restored: T,
        update_error: anyhow::Error,
    },
}

fn ensure_idle_for_input_change(
    state: &RecordingState,
    input_name: &str,
) -> Result<(), anyhow::Error> {
    if matches!(state, RecordingState::Idle) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Cannot change the {input_name} while recording"
        ))
    }
}

/// Serializes every operation that can open, close, or reconfigure the input
/// stream. Device/channel changes keep this guard through their transaction;
/// a mode transition must take the same guard before it can start an
/// always-on stream from a settings snapshot.
fn lock_input_reconfiguration(state: &Mutex<RecordingState>) -> MutexGuard<'_, RecordingState> {
    state.lock().unwrap()
}

/// Applies a microphone-mode change as one transaction. The runtime stream is
/// reconfigured first; only then do we expose the new mode and persist it.
///
/// Keeping this independent from Tauri lets the exact manager transition be
/// exercised deterministically with controlled stream and persistence seams.
fn update_mode_transaction(
    state: &Mutex<RecordingState>,
    mode: &Mutex<MicrophoneMode>,
    new_mode: MicrophoneMode,
    stop_stream: impl FnOnce(),
    start_stream: impl FnOnce() -> Result<(), anyhow::Error>,
    persist_mode: impl FnOnce(&MicrophoneMode),
    #[cfg(test)] before_reconfiguration_lock: Option<&dyn Fn()>,
) -> Result<(), anyhow::Error> {
    #[cfg(test)]
    if let Some(before_reconfiguration_lock) = before_reconfiguration_lock {
        before_reconfiguration_lock();
    }

    let state = lock_input_reconfiguration(state);
    let current_mode = mode.lock().unwrap().clone();

    match (current_mode, &new_mode) {
        (MicrophoneMode::AlwaysOn, MicrophoneMode::OnDemand) => {
            if matches!(*state, RecordingState::Idle) {
                stop_stream();
            }
        }
        (MicrophoneMode::OnDemand, MicrophoneMode::AlwaysOn) => {
            start_stream()?;
        }
        _ => {}
    }

    *mode.lock().unwrap() = new_mode.clone();
    persist_mode(&new_mode);
    Ok(())
}

/// Stops an already-open idle stream, starts it with the proposed setting, and
/// restores the previous setting if the proposed stream cannot open.
///
/// The closures make the CoreAudio boundary explicit and keep the rollback
/// testable without depending on a physical input device.
fn restart_open_stream_transaction<T>(
    stop_stream: impl FnOnce(),
    start_updated_stream: impl FnOnce() -> Result<T, anyhow::Error>,
    restore_previous_stream: impl FnOnce() -> Result<T, anyhow::Error>,
) -> Result<StreamRestartOutcome<T>, anyhow::Error> {
    stop_stream();

    match start_updated_stream() {
        Ok(updated) => Ok(StreamRestartOutcome::Updated(updated)),
        Err(update_error) => match restore_previous_stream() {
            Ok(restored) => Ok(StreamRestartOutcome::RestoredAfterUpdateFailure {
                restored,
                update_error,
            }),
            Err(restore_error) => Err(anyhow::anyhow!(
                "Failed to apply input change ({update_error}); failed to restore the previous input ({restore_error})"
            )),
        },
    }
}

/* ──────────────────────────────────────────────────────────────── */

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
    /* ---------- construction ------------------------------------------------ */

    pub fn new(app: &tauri::AppHandle) -> Result<Self, anyhow::Error> {
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
            capture_generation: Arc::new(AtomicU64::new(0)),
            cached_device: Arc::new(Mutex::new(None)),
        };

        // The persisted AlwaysOn value was committed by a previous successful
        // transition, so startup opens the stream without writing or emitting
        // it again. If opening fails, construction fails and no live manager
        // exposes an uncommitted runtime mode.
        if matches!(mode, MicrophoneMode::AlwaysOn) {
            manager.start_microphone_stream()?;
        }

        Ok(manager)
    }

    /* ---------- helper methods --------------------------------------------- */

    /// The persisted microphone preference currently in effect. Only runs the
    /// clamshell probe (an `ioreg` subprocess, ~10-20ms) when a clamshell
    /// microphone is actually configured.
    fn desired_microphone(&self, settings: &AppSettings) -> DesiredMicrophone {
        if let Some(clamshell_microphone) = &settings.clamshell_microphone {
            let clamshell_started = Instant::now();
            let is_clamshell = clamshell::is_clamshell().unwrap_or(false);
            debug!(
                "device resolve: clamshell_check={:?} (clamshell={})",
                clamshell_started.elapsed(),
                is_clamshell
            );
            if is_clamshell {
                return DesiredMicrophone::Clamshell(clamshell_microphone.clone());
            }
        }
        match &settings.selected_microphone {
            Some(name) => DesiredMicrophone::Selected(name.clone()),
            None => DesiredMicrophone::Default,
        }
    }

    pub fn invalidate_device_cache(&self) {
        *self.cached_device.lock().unwrap() = None;
    }

    fn resolve_microphone_device(&self, settings: &AppSettings) -> MicrophoneResolution {
        let desired = self.desired_microphone(settings);
        let (device_name, selected_microphone) = match desired {
            DesiredMicrophone::Default => {
                debug!("device resolve: no mic configured -> system default");
                return MicrophoneResolution {
                    device: None,
                    unavailable_selected_microphone: None,
                };
            }
            DesiredMicrophone::Selected(name) => (name.clone(), Some(name)),
            DesiredMicrophone::Clamshell(name) => (name, None),
        };

        // Cache hit: skip the full enumeration. A stale device (unplugged)
        // fails at open, where the caller invalidates and retries fresh.
        if let Some((cached_name, device)) = self.cached_device.lock().unwrap().as_ref() {
            if *cached_name == device_name {
                debug!("device resolve: cache hit for '{}'", device_name);
                return MicrophoneResolution {
                    device: Some(device.clone()),
                    unavailable_selected_microphone: None,
                };
            }
        }

        // Only report a selected microphone as unavailable when enumeration
        // itself succeeded. A backend enumeration error may be transient and
        // must not erase the user's persisted preference.
        let enumerate_started = Instant::now();
        let (device, enumeration_succeeded) = match list_input_devices() {
            Ok(devices) => (
                devices
                    .into_iter()
                    .find(|d| d.name == device_name)
                    .map(|d| d.device),
                true,
            ),
            Err(e) => {
                debug!("Failed to list devices, using default: {}", e);
                (None, false)
            }
        };
        debug!(
            "device resolve: enumerate={:?} (found={})",
            enumerate_started.elapsed(),
            device.is_some()
        );
        if let Some(d) = &device {
            *self.cached_device.lock().unwrap() = Some((device_name, d.clone()));
        }

        let unavailable_selected_microphone = if enumeration_succeeded && device.is_none() {
            selected_microphone
        } else {
            None
        };
        MicrophoneResolution {
            device,
            unavailable_selected_microphone,
        }
    }

    /// Keep persisted settings and the UI aligned with a successful runtime
    /// fallback. Re-read first so recovery cannot clear a microphone the user
    /// selected concurrently while the stream was being rebuilt.
    fn persist_default_microphone_after_fallback(&self, unavailable_name: &str) {
        let mut settings = get_settings(&self.app_handle);
        if settings.selected_microphone.as_deref() != Some(unavailable_name) {
            return;
        }

        settings.selected_microphone = None;
        write_settings(&self.app_handle, settings);
        self.emit_default_microphone_fallback();
    }

    fn emit_default_microphone_fallback(&self) {
        let _ = self.app_handle.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "selected_microphone",
                "value": "Default"
            }),
        );
    }

    fn schedule_lazy_close(&self) {
        let gen = self.close_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let app = self.app_handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(STREAM_IDLE_TIMEOUT);
            let rm = app.state::<Arc<AudioRecordingManager>>();
            // Hold state lock across the check AND close to serialize against
            // try_start_recording, preventing a race where the stream is closed
            // under an active recording.
            let state = rm.state.lock().unwrap();
            if rm.close_generation.load(Ordering::SeqCst) == gen
                && matches!(*state, RecordingState::Idle)
            {
                // stop_microphone_stream does not acquire the state lock,
                // so holding it here is safe (no deadlock).
                info!(
                    "Closing idle microphone stream after {:?}",
                    STREAM_IDLE_TIMEOUT
                );
                rm.stop_microphone_stream();
            }
        });
    }

    /* ---------- microphone life-cycle -------------------------------------- */

    /// Applies mute if mute_while_recording is enabled and stream is open.
    /// Snapshots the system's prior mute state first so `remove_mute` can
    /// restore it instead of unconditionally unmuting.
    pub fn apply_mute(&self) {
        let settings = get_settings(&self.app_handle);
        if !settings.mute_while_recording {
            return;
        }

        // Lock order: is_open before mute_state (matches stop_microphone_stream).
        let is_open = self.is_open.lock().unwrap();
        let mut mute_guard = self.mute_state.lock().unwrap();
        // Already muted this session — don't re-snapshot, or a duplicate/late
        // apply would overwrite prev_muted with our own forced-muted state and
        // strand audio muted on stop.
        if mute_guard.did_mute {
            return;
        }
        if *is_open {
            mute_guard.prev_muted = get_mute();
            set_mute(true);
            mute_guard.did_mute = true;
            debug!("Mute applied (prev_muted={:?})", mute_guard.prev_muted);
        }
    }

    /// Removes mute if it was applied, restoring the system's prior mute state
    /// (a system already muted before recording stays muted).
    pub fn remove_mute(&self) {
        let mut mute_guard = self.mute_state.lock().unwrap();
        if mute_guard.did_mute {
            restore_mute(mute_guard.prev_muted);
            mute_guard.did_mute = false;
            debug!(
                "Mute removed (restored prev_muted={:?})",
                mute_guard.prev_muted
            );
        }
    }

    pub fn ensure_recorder(&self) -> Result<(), anyhow::Error> {
        let selected_channel = get_settings(&self.app_handle).selected_channel;
        self.ensure_recorder_with_channel(selected_channel)
    }

    fn ensure_recorder_with_channel(
        &self,
        selected_channel: Option<u16>,
    ) -> Result<(), anyhow::Error> {
        let mut recorder_opt = self.recorder.lock().unwrap();
        if recorder_opt.is_none() {
            *recorder_opt = Some(create_audio_recorder(&self.app_handle, selected_channel)?);
        }
        Ok(())
    }

    pub fn start_microphone_stream(&self) -> Result<(), anyhow::Error> {
        let settings = get_settings(&self.app_handle);
        if let Some(unavailable_name) = self.start_microphone_stream_with_settings(&settings)? {
            // Do this only after the default stream opened successfully. A
            // failed fallback must not erase the user's microphone preference.
            self.persist_default_microphone_after_fallback(&unavailable_name);
        }
        Ok(())
    }

    /// Opens the stream for an explicit settings snapshot. The caller owns any
    /// persistence so it can commit the setting only after CoreAudio accepts it.
    fn start_microphone_stream_with_settings(
        &self,
        settings: &AppSettings,
    ) -> Result<Option<String>, anyhow::Error> {
        let mut open_flag = self.is_open.lock().unwrap();
        if *open_flag {
            // `is_open` only records that we opened a stream at some point, not
            // that one is still running. If capture has since failed (mic
            // unplugged mid-session, USB dropout), rebuild it before the next
            // recording instead of handing the caller a stalled recorder.
            let needs_reopen = self
                .recorder
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|rec| rec.needs_reopen());

            if !needs_reopen {
                // trace, not debug: with the aliveness check in
                // try_start_recording this now fires on every keypress in
                // always-on mode.
                trace!("Microphone stream already active");
                return Ok(None);
            }

            warn!("Microphone stream is no longer running (device disconnected?); reopening");

            // Torn down inline rather than via stop_microphone_stream(), which
            // takes the `is_open` lock we are already holding.
            {
                let mut mute_guard = self.mute_state.lock().unwrap();
                if mute_guard.did_mute {
                    restore_mute(mute_guard.prev_muted);
                    mute_guard.did_mute = false;
                }
            }
            if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
                let _ = rec.close();
            }
            *self.is_recording.lock().unwrap() = false;
            *open_flag = false;
            self.invalidate_device_cache();
            // Fall through to the same fresh resolution and fallback path used
            // when an on-demand stream opens after its device was unplugged.
        }

        let start_time = Instant::now();

        // Don't mute immediately - caller will handle muting after audio feedback.
        // The previous stream restored audio on close, so did_mute should already
        // be false here; if it somehow isn't, restore rather than just clearing the
        // flag, which would strand system audio muted.
        {
            let mut mute_guard = self.mute_state.lock().unwrap();
            if mute_guard.did_mute {
                restore_mute(mute_guard.prev_muted);
                mute_guard.did_mute = false;
            }
        }

        // Get the selected device from settings, considering clamshell mode.
        // No pre-flight enumeration here: when nothing is configured the
        // recorder resolves the system default itself, and a machine with no
        // input devices at all fails inside open() with the same
        // "No input device found" error this used to check for.
        let resolve_started = Instant::now();
        let mut resolution = self.resolve_microphone_device(settings);
        let resolve_elapsed = resolve_started.elapsed();

        let recorder_started = Instant::now();
        self.ensure_recorder_with_channel(settings.selected_channel)?;
        let recorder_elapsed = recorder_started.elapsed();

        let open_started = Instant::now();
        let mut recorder_opt = self.recorder.lock().unwrap();
        if let Some(rec) = recorder_opt.as_mut() {
            if let Err(first_err) = rec.open(resolution.device.clone()) {
                // A cached device or config may have gone stale (unplugged,
                // rate/format changed). Re-resolve from a fresh enumeration and
                // retry once before surfacing the error.
                warn!("Recorder open failed ({first_err}); re-resolving device and retrying once");
                self.invalidate_device_cache();
                resolution = self.resolve_microphone_device(settings);
                rec.open(resolution.device.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to open recorder: {}", e))?;
            }
        }
        debug!(
            "mic stream breakdown: device_resolve={:?} recorder_ensure={:?} open={:?}",
            resolve_elapsed,
            recorder_elapsed,
            open_started.elapsed()
        );
        drop(recorder_opt);

        *open_flag = true;
        // This timing covers through cpal's stream.play() returning — i.e. the
        // point cpal surfaces as "stream running." It does NOT guarantee the
        // host audio device is producing samples yet; the first input callback
        // fires asynchronously one buffer period later (hardware dependent,
        // typically ~10–200ms on macOS, longer on Bluetooth/USB).
        info!(
            "Microphone stream initialized in {:?}",
            start_time.elapsed()
        );
        Ok(resolution.unavailable_selected_microphone)
    }

    pub fn stop_microphone_stream(&self) {
        let mut open_flag = self.is_open.lock().unwrap();
        if !*open_flag {
            return;
        }

        {
            let mut mute_guard = self.mute_state.lock().unwrap();
            if mute_guard.did_mute {
                restore_mute(mute_guard.prev_muted);
            }
            mute_guard.did_mute = false;
        }

        if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
            // If still recording, stop first.
            if *self.is_recording.lock().unwrap() {
                let _ = rec.stop();
                *self.is_recording.lock().unwrap() = false;
            }
            let _ = rec.close();
        }

        *open_flag = false;
        debug!("Microphone stream stopped");
    }

    /* ---------- mode switching --------------------------------------------- */

    pub fn update_mode(&self, new_mode: MicrophoneMode) -> Result<(), anyhow::Error> {
        update_mode_transaction(
            &self.state,
            &self.mode,
            new_mode,
            || {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.stop_microphone_stream();
            },
            || {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.start_microphone_stream()
            },
            |mode| self.persist_microphone_mode(mode),
            #[cfg(test)]
            None,
        )
    }

    fn persist_microphone_mode(&self, mode: &MicrophoneMode) {
        let always_on = matches!(mode, MicrophoneMode::AlwaysOn);
        let mut settings = get_settings(&self.app_handle);
        settings.always_on_microphone = always_on;
        write_settings(&self.app_handle, settings);
        let _ = self.app_handle.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "always_on_microphone",
                "value": always_on,
            }),
        );
    }

    /* ---------- recording --------------------------------------------------- */

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
    }

    pub fn try_start_recording(&self, binding_id: &str) -> Result<RecordingReadiness, String> {
        let mut state = self.state.lock().unwrap();

        if let RecordingState::Idle = *state {
            // Cancel any pending lazy close (no-op in always-on mode, where
            // closes are never scheduled).
            self.close_generation.fetch_add(1, Ordering::SeqCst);
            // Opens the stream in on-demand mode. In always-on mode the stream
            // is normally already open and this is a cheap aliveness check —
            // but if the capture worker died (device disconnect), it rebuilds
            // the stream instead of leaving every subsequent start wedged on
            // "Recorder not available".
            if let Err(e) = self.start_microphone_stream() {
                let msg = format!("{e}");
                error!("Failed to open microphone stream: {msg}");
                return Err(msg);
            }

            if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                match rec.start() {
                    Ok(receiver) => {
                        let generation = self.capture_generation.fetch_add(1, Ordering::AcqRel) + 1;
                        *self.is_recording.lock().unwrap() = true;
                        self.set_state(
                            &mut state,
                            RecordingState::Recording {
                                binding_id: binding_id.to_string(),
                            },
                        );
                        debug!("Recording requested for binding {binding_id}");
                        return Ok(RecordingReadiness {
                            receiver,
                            generation,
                        });
                    }
                    Err(error) => return Err(format!("Failed to start recorder: {error}")),
                }
            }
            Err("Recorder not available".to_string())
        } else {
            Err("Already recording".to_string())
        }
    }

    pub fn update_selected_device(
        &self,
        selected_microphone: Option<String>,
    ) -> Result<(), anyhow::Error> {
        // Serialize against recording start/stop. Restarting an active capture
        // would discard its samples and leave the manager's recording state out
        // of sync with the new recorder.
        let state = lock_input_reconfiguration(&self.state);
        ensure_idle_for_input_change(&state, "microphone")?;

        let previous_settings = get_settings(&self.app_handle);
        let mut updated_settings = previous_settings.clone();
        updated_settings.selected_microphone = selected_microphone;

        // Device settings changed; re-enumerate the device before a later
        // on-demand open. When the stream is already open, first prove the
        // replacement can start, then write the effective preference.
        self.invalidate_device_cache();
        let was_open = *self.is_open.lock().unwrap();
        if !was_open {
            write_settings(&self.app_handle, updated_settings);
            return Ok(());
        }

        match restart_open_stream_transaction(
            || {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.stop_microphone_stream();
            },
            || self.start_microphone_stream_with_settings(&updated_settings),
            || {
                self.invalidate_device_cache();
                self.start_microphone_stream_with_settings(&previous_settings)
            },
        )? {
            StreamRestartOutcome::Updated(unavailable_name) => {
                let fell_back_to_default = unavailable_name.is_some();
                if fell_back_to_default {
                    updated_settings.selected_microphone = None;
                }
                write_settings(&self.app_handle, updated_settings);
                if fell_back_to_default {
                    self.emit_default_microphone_fallback();
                }
                Ok(())
            }
            StreamRestartOutcome::RestoredAfterUpdateFailure {
                restored,
                update_error,
            } => {
                // The original device may have disappeared while the failed
                // replacement was being opened. If recovery fell back to the
                // system default, record that live state instead of reviving a
                // stale microphone preference.
                let restored_default_microphone = restored.is_some();
                if restored_default_microphone {
                    let mut restored_settings = previous_settings;
                    restored_settings.selected_microphone = None;
                    write_settings(&self.app_handle, restored_settings);
                    self.emit_default_microphone_fallback();
                }
                Err(update_error)
            }
        }
    }

    pub fn update_selected_channel(
        &self,
        selected_channel: Option<u16>,
    ) -> Result<(), anyhow::Error> {
        // Serialize against recording start/stop. Restarting an active capture
        // would discard its samples and leave the manager's recording state out
        // of sync with the new recorder.
        let state = lock_input_reconfiguration(&self.state);
        ensure_idle_for_input_change(&state, "input channel")?;

        let previous_settings = get_settings(&self.app_handle);
        let previous_channel = previous_settings.selected_channel;
        let mut updated_settings = previous_settings.clone();
        updated_settings.selected_channel = selected_channel;
        let was_open = *self.is_open.lock().unwrap();
        if !was_open {
            if let Some(recorder) = self.recorder.lock().unwrap().as_mut() {
                recorder.set_selected_channel(selected_channel);
            }
            write_settings(&self.app_handle, updated_settings);
            return Ok(());
        }

        match restart_open_stream_transaction(
            || {
                self.close_generation.fetch_add(1, Ordering::SeqCst);
                self.stop_microphone_stream();
            },
            || {
                if let Some(recorder) = self.recorder.lock().unwrap().as_mut() {
                    recorder.set_selected_channel(selected_channel);
                }
                self.start_microphone_stream_with_settings(&updated_settings)
            },
            || {
                if let Some(recorder) = self.recorder.lock().unwrap().as_mut() {
                    recorder.set_selected_channel(previous_channel);
                }
                self.invalidate_device_cache();
                self.start_microphone_stream_with_settings(&previous_settings)
            },
        )? {
            StreamRestartOutcome::Updated(unavailable_name) => {
                let fell_back_to_default = unavailable_name.is_some();
                if fell_back_to_default {
                    updated_settings.selected_microphone = None;
                }
                write_settings(&self.app_handle, updated_settings);
                if fell_back_to_default {
                    self.emit_default_microphone_fallback();
                }
                Ok(())
            }
            StreamRestartOutcome::RestoredAfterUpdateFailure {
                restored,
                update_error,
            } => {
                let restored_default_microphone = restored.is_some();
                if restored_default_microphone {
                    let mut restored_settings = previous_settings;
                    restored_settings.selected_microphone = None;
                    write_settings(&self.app_handle, restored_settings);
                    self.emit_default_microphone_fallback();
                }
                Err(update_error)
            }
        }
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

    pub fn stop_recording(&self, binding_id: &str, cancel_generation: u64) -> Option<Vec<f32>> {
        self.invalidate_recording_readiness();
        let mut state = self.state.lock().unwrap();

        match *state {
            RecordingState::Recording {
                binding_id: ref active,
            } if active == binding_id => {
                self.set_state(&mut state, RecordingState::Stopping);
                drop(state);

                // Optionally keep recording for a bit longer to capture trailing audio.
                let settings = get_settings(&self.app_handle);
                let buffer_ms = settings.extra_recording_buffer_ms;
                if buffer_ms > 0 {
                    debug!(
                        "Extra recording buffer: sleeping {}ms before stopping",
                        buffer_ms
                    );
                    let started = Instant::now();
                    let buffer = Duration::from_millis(buffer_ms);
                    while started.elapsed() < buffer {
                        if self.was_cancelled_since(cancel_generation) {
                            debug!("Recording stop cancelled during extra buffer");
                            break;
                        }
                        let remaining = buffer.saturating_sub(started.elapsed());
                        std::thread::sleep(remaining.min(Duration::from_millis(25)));
                    }
                }

                let samples = if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                    match rec.stop() {
                        Ok(buf) => buf,
                        Err(e) => {
                            error!("stop() failed: {e}");
                            Vec::new()
                        }
                    }
                } else {
                    error!("Recorder not available");
                    Vec::new()
                };

                *self.is_recording.lock().unwrap() = false;
                self.set_state(&mut self.state.lock().unwrap(), RecordingState::Idle);

                // In on-demand mode, close the mic (lazily if the setting is enabled)
                if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }

                if self.was_cancelled_since(cancel_generation) {
                    debug!("Recording stop cancelled; discarding captured samples");
                    return None;
                }

                // Pad if very short
                let s_len = samples.len();
                // debug!("Got {} samples", s_len);
                if s_len < TARGET_SAMPLE_RATE && s_len > 0 {
                    let mut padded = samples;
                    padded.resize(TARGET_SAMPLE_RATE * 5 / 4, 0.0);
                    Some(padded)
                } else {
                    Some(samples)
                }
            }
            _ => None,
        }
    }
    pub fn is_recording(&self) -> bool {
        // Lock-free: mirrors the `state` {Recording, Stopping} membership via
        // an atomic maintained by `set_state()`. Polled from the webview/main
        // thread, so it MUST NOT take the `state` mutex (a worker can hold it
        // across a slow CoreAudio open/close → main-thread deadlock / UI
        // freeze).
        self.recording_active.load(Ordering::SeqCst)
    }

    /// Cancel any ongoing recording without returning audio samples
    pub fn cancel_recording(&self) {
        self.invalidate_recording_readiness();
        self.cancel_generation.fetch_add(1, Ordering::AcqRel);
        let mut state = self.state.lock().unwrap();

        match *state {
            RecordingState::Recording { .. } => {
                self.set_state(&mut state, RecordingState::Idle);
                drop(state);

                if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                    let _ = rec.stop(); // Discard the result
                }

                *self.is_recording.lock().unwrap() = false;

                // In on-demand mode, close the mic (lazily if the setting is enabled)
                if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }
            }
            RecordingState::Stopping => {
                debug!("Cancellation requested while recording is stopping");
            }
            RecordingState::Idle => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Arc;

    #[test]
    fn input_changes_require_an_idle_recording_state() {
        assert!(ensure_idle_for_input_change(&RecordingState::Idle, "microphone").is_ok());

        let recording = ensure_idle_for_input_change(
            &RecordingState::Recording {
                binding_id: "main".to_string(),
            },
            "microphone",
        )
        .unwrap_err();
        assert_eq!(
            recording.to_string(),
            "Cannot change the microphone while recording"
        );

        let stopping =
            ensure_idle_for_input_change(&RecordingState::Stopping, "input channel").unwrap_err();
        assert_eq!(
            stopping.to_string(),
            "Cannot change the input channel while recording"
        );
    }

    #[test]
    fn on_demand_to_always_on_waits_for_device_or_channel_transaction_commit() {
        for input_name in ["microphone", "input channel"] {
            let state = Arc::new(Mutex::new(RecordingState::Idle));
            let configured_input = Arc::new(Mutex::new("previous input"));
            let mode = Arc::new(Mutex::new(MicrophoneMode::OnDemand));
            let persisted_modes = Arc::new(Mutex::new(Vec::new()));
            let (transaction_started_tx, transaction_started_rx) = mpsc::sync_channel(0);
            let (commit_transaction_tx, commit_transaction_rx) = mpsc::sync_channel(0);

            let state_for_transaction = Arc::clone(&state);
            let configured_input_for_transaction = Arc::clone(&configured_input);
            let transaction = std::thread::spawn(move || {
                let state = lock_input_reconfiguration(&state_for_transaction);
                ensure_idle_for_input_change(&state, input_name).unwrap();
                transaction_started_tx.send(()).unwrap();

                commit_transaction_rx.recv().unwrap();
                *configured_input_for_transaction.lock().unwrap() = "updated input";
            });

            transaction_started_rx.recv().unwrap();

            let state_for_mode = Arc::clone(&state);
            let configured_input_for_mode = Arc::clone(&configured_input);
            let mode_for_transition = Arc::clone(&mode);
            let persisted_modes_for_transition = Arc::clone(&persisted_modes);
            let (mode_before_lock_tx, mode_before_lock_rx) = mpsc::sync_channel(0);
            let (stream_started_with_tx, stream_started_with_rx) = mpsc::channel();
            let mode_transition = std::thread::spawn(move || {
                let before_reconfiguration_lock = || mode_before_lock_tx.send(()).unwrap();
                update_mode_transaction(
                    &state_for_mode,
                    &mode_for_transition,
                    MicrophoneMode::AlwaysOn,
                    || panic!("OnDemand -> AlwaysOn must not stop the stream"),
                    || {
                        stream_started_with_tx
                            .send(*configured_input_for_mode.lock().unwrap())
                            .unwrap();
                        Ok(())
                    },
                    |mode| {
                        persisted_modes_for_transition
                            .lock()
                            .unwrap()
                            .push(matches!(mode, MicrophoneMode::AlwaysOn))
                    },
                    Some(&before_reconfiguration_lock),
                )
                .unwrap();
            });

            // The exact update-mode transaction has reached the same lock that
            // the input transaction holds. It therefore cannot snapshot or
            // open until this controlled commit finishes.
            mode_before_lock_rx.recv().unwrap();
            commit_transaction_tx.send(()).unwrap();
            transaction.join().unwrap();
            mode_transition.join().unwrap();
            assert_eq!(stream_started_with_rx.recv().unwrap(), "updated input");
            assert!(matches!(*mode.lock().unwrap(), MicrophoneMode::AlwaysOn));
            assert_eq!(*persisted_modes.lock().unwrap(), [true]);
        }
    }

    #[test]
    fn always_on_open_failure_keeps_mode_and_persistence_on_demand() {
        let state = Mutex::new(RecordingState::Idle);
        let mode = Mutex::new(MicrophoneMode::OnDemand);
        let persisted_modes = Mutex::new(Vec::new());

        let result = update_mode_transaction(
            &state,
            &mode,
            MicrophoneMode::AlwaysOn,
            || panic!("OnDemand -> AlwaysOn must not stop the stream"),
            || Err(anyhow::anyhow!("input unavailable")),
            |mode| {
                persisted_modes
                    .lock()
                    .unwrap()
                    .push(matches!(mode, MicrophoneMode::AlwaysOn))
            },
            None,
        );

        assert_eq!(result.unwrap_err().to_string(), "input unavailable");
        assert!(matches!(*mode.lock().unwrap(), MicrophoneMode::OnDemand));
        assert!(persisted_modes.lock().unwrap().is_empty());
    }

    #[test]
    fn stream_restart_uses_the_updated_input_after_closing_the_old_stream() {
        let steps = RefCell::new(Vec::new());
        let outcome = restart_open_stream_transaction(
            || steps.borrow_mut().push("stop"),
            || {
                steps.borrow_mut().push("start-updated");
                Ok::<_, anyhow::Error>("updated")
            },
            || unreachable!("the previous stream must not be reopened after a successful update"),
        )
        .unwrap();

        assert_eq!(*steps.borrow(), ["stop", "start-updated"]);
        assert!(matches!(outcome, StreamRestartOutcome::Updated("updated")));
    }

    #[test]
    fn stream_restart_restores_the_previous_input_when_the_update_fails() {
        let steps = RefCell::new(Vec::new());
        let outcome = restart_open_stream_transaction(
            || steps.borrow_mut().push("stop"),
            || {
                steps.borrow_mut().push("start-updated");
                Err(anyhow::anyhow!("new input rejected"))
            },
            || {
                steps.borrow_mut().push("restore-previous");
                Ok::<_, anyhow::Error>("previous")
            },
        )
        .unwrap();

        assert_eq!(
            *steps.borrow(),
            ["stop", "start-updated", "restore-previous"]
        );
        match outcome {
            StreamRestartOutcome::RestoredAfterUpdateFailure {
                restored,
                update_error,
            } => {
                assert_eq!(restored, "previous");
                assert_eq!(update_error.to_string(), "new input rejected");
            }
            StreamRestartOutcome::Updated(_) => {
                panic!("expected the previous stream to be restored")
            }
        }
    }

    #[test]
    fn stream_restart_reports_when_neither_input_can_be_opened() {
        let result = restart_open_stream_transaction::<()>(
            || {},
            || Err(anyhow::anyhow!("new input rejected")),
            || Err(anyhow::anyhow!("previous input rejected")),
        );

        match result {
            Ok(_) => panic!("expected both stream opens to fail"),
            Err(error) => assert_eq!(
                error.to_string(),
                "Failed to apply input change (new input rejected); failed to restore the previous input (previous input rejected)"
            ),
        }
    }
}
