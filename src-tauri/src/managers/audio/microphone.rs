use crate::audio_toolkit::{list_input_devices, AudioRecorder};
use crate::helpers::clamshell;
use crate::settings::{get_settings, update_settings, AppSettings};
use crate::utils;
use log::{debug, info, trace, warn};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

use super::*;

pub(super) const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// A late fallback must not clear a microphone chosen since its stream started
/// opening. Call this against the current settings inside a mutation transaction.
pub(super) fn clear_unavailable_microphone(
    settings: &mut AppSettings,
    unavailable_name: &str,
) -> bool {
    if settings.selected_microphone.as_deref() != Some(unavailable_name) {
        return false;
    }
    settings.selected_microphone = None;
    true
}

/// The persisted microphone preference currently in effect. Clamshell and
/// regular selections are kept distinct so losing a clamshell-only device does
/// not erase the user's normal microphone preference.
pub(super) enum DesiredMicrophone {
    Default,
    Selected(String),
    Clamshell(String),
}

/// The persisted microphone preference currently in effect. Only runs the
/// clamshell probe (an `ioreg` subprocess, ~10-20ms) when a clamshell
/// microphone is actually configured.
pub(super) fn desired_microphone(settings: &AppSettings) -> DesiredMicrophone {
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

/// The name of the microphone dictation records from, `None` for the system
/// default. Meetings record the same one.
pub fn preferred_microphone(settings: &AppSettings) -> Option<String> {
    match desired_microphone(settings) {
        DesiredMicrophone::Default => None,
        DesiredMicrophone::Selected(name) | DesiredMicrophone::Clamshell(name) => Some(name),
    }
}

/// Result of resolving the persisted preference to a live cpal device.
/// `device: None` means cpal should open the system default. The unavailable
/// name is populated only when enumeration succeeded and confirmed that the
/// user's regular selected microphone is missing.
pub(super) struct MicrophoneResolution {
    pub(super) device: Option<cpal::Device>,
    pub(super) unavailable_selected_microphone: Option<String>,
}

pub(super) fn create_audio_recorder(
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
pub(super) enum StreamRestartOutcome<T> {
    Updated(T),
    RestoredAfterUpdateFailure {
        restored: T,
        update_error: anyhow::Error,
    },
}

pub(super) fn ensure_idle_for_input_change(
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
pub(super) fn lock_input_reconfiguration(
    state: &Mutex<RecordingState>,
) -> MutexGuard<'_, RecordingState> {
    state.lock().unwrap()
}

/// Applies a microphone-mode change as one transaction. The runtime stream is
/// reconfigured first; only then do we expose the new mode and persist it.
///
/// Keeping this independent from Tauri lets the exact manager transition be
/// exercised deterministically with controlled stream and persistence seams.
pub(super) fn update_mode_transaction(
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
            // A recording already holds an open stream, and stop_recording
            // keeps it open once it reads AlwaysOn. Starting it here could
            // reopen it under the recording and drop what it captured.
            if matches!(*state, RecordingState::Idle) {
                start_stream()?;
            }
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
pub(super) fn restart_open_stream_transaction<T>(
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

impl AudioRecordingManager {
    pub(super) fn resolve_microphone_device(&self, settings: &AppSettings) -> MicrophoneResolution {
        let desired = desired_microphone(settings);
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
    /// fallback. Check and clear in one transaction so recovery cannot clear a
    /// microphone the user selected while the stream was being rebuilt.
    pub(super) fn persist_default_microphone_after_fallback(&self, unavailable_name: &str) {
        let changed = update_settings(&self.app_handle, |settings| {
            clear_unavailable_microphone(settings, unavailable_name)
        });
        if changed {
            self.emit_default_microphone_fallback();
        }
    }

    pub(super) fn emit_default_microphone_fallback(&self) {
        let _ = self.app_handle.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "selected_microphone",
                "value": "Default"
            }),
        );
    }

    pub(super) fn schedule_lazy_close(&self) {
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

    pub fn ensure_recorder(&self) -> Result<(), anyhow::Error> {
        let selected_channel = get_settings(&self.app_handle).selected_channel;
        self.ensure_recorder_with_channel(selected_channel)
    }

    pub(super) fn ensure_recorder_with_channel(
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
    pub(super) fn start_microphone_stream_with_settings(
        &self,
        settings: &AppSettings,
    ) -> Result<Option<String>, anyhow::Error> {
        let mut open_flag = self.is_open.lock().unwrap();
        if *open_flag {
            // `is_open` only records that we opened a stream at some point, not
            // that one is still running. If capture has since failed (mic
            // unplugged mid-session, USB dropout) or the system default it was
            // opened on is now another microphone, rebuild it before the next
            // recording instead of handing the caller a stalled recorder.
            let needs_reopen = self
                .recorder
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|rec| rec.needs_reopen());

            if !needs_reopen {
                // trace, not debug: the aliveness check in try_start_recording
                // reaches this on every keypress in always-on mode.
                trace!("Microphone stream already active");
                return Ok(None);
            }

            warn!("Microphone stream is no longer usable; reopening");

            // Torn down inline rather than via stop_microphone_stream(), which
            // takes the `is_open` lock we are already holding.
            self.mute_state.lock().unwrap().release(restore_mute);
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
        self.mute_state.lock().unwrap().release(restore_mute);

        // No pre-flight enumeration here: when nothing is configured the
        // recorder resolves the system default itself, and a machine with no
        // input devices at all fails inside open() with "No input device found".
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

        self.mute_state.lock().unwrap().release(restore_mute);

        if let Some(rec) = self.recorder.lock().unwrap().as_mut() {
            if *self.is_recording.lock().unwrap() {
                let _ = rec.stop();
                *self.is_recording.lock().unwrap() = false;
            }
            let _ = rec.close();
        }

        *open_flag = false;
        debug!("Microphone stream stopped");
    }

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

    pub(super) fn persist_microphone_mode(&self, mode: &MicrophoneMode) {
        let always_on = matches!(mode, MicrophoneMode::AlwaysOn);
        update_settings(&self.app_handle, |settings| {
            settings.always_on_microphone = always_on;
        });
        let _ = self.app_handle.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "always_on_microphone",
                "value": always_on,
            }),
        );
    }
}
