use crate::audio_feedback;
use crate::audio_toolkit::audio::ReleaseTailRequest;
use crate::audio_toolkit::release_tail::{self, TailConfig};
use crate::audio_toolkit::Capture;
use crate::settings::{get_settings, update_settings};
use log::{debug, error};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::*;

impl AudioRecordingManager {
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
            update_settings(&self.app_handle, |settings| {
                settings.selected_microphone = updated_settings.selected_microphone;
            });
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
                update_settings(&self.app_handle, |settings| {
                    settings.selected_microphone = updated_settings.selected_microphone;
                });
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
                if let Some(unavailable_name) = restored {
                    self.persist_default_microphone_after_fallback(&unavailable_name);
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
            update_settings(&self.app_handle, |settings| {
                settings.selected_channel = selected_channel;
            });
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
                let cleared_microphone = update_settings(&self.app_handle, |settings| {
                    settings.selected_channel = selected_channel;
                    unavailable_name
                        .as_deref()
                        .is_some_and(|name| clear_unavailable_microphone(settings, name))
                });
                if cleared_microphone {
                    self.emit_default_microphone_fallback();
                }
                Ok(())
            }
            StreamRestartOutcome::RestoredAfterUpdateFailure {
                restored,
                update_error,
            } => {
                if let Some(unavailable_name) = restored {
                    self.persist_default_microphone_after_fallback(&unavailable_name);
                }
                Err(update_error)
            }
        }
    }

    pub fn stop_recording(&self, binding_id: &str, cancel_generation: u64) -> Option<Capture> {
        let mut state = self.state.lock().unwrap();

        // This check is deliberately inside the state lock and before any
        // readiness invalidation or recorder transition. A cancelled stop
        // worker can be scheduled after a newer recording reuses the same
        // binding id; it must then be a no-op rather than stop that recording.
        if !can_claim_stop(
            &state,
            binding_id,
            cancel_generation,
            self.cancel_generation(),
        ) {
            return None;
        }

        match *state {
            RecordingState::Recording {
                binding_id: ref active,
            } if active == binding_id => {
                self.invalidate_recording_readiness();
                self.set_state(&mut state, RecordingState::Stopping);
                drop(state);

                // Keep recording for a moment after the release so the last
                // word is not cut; a cancel ends the wait at once.
                let tail = self.release_tail_request(cancel_generation);

                let Capture { samples, mic_lost } =
                    if let Some(rec) = self.recorder.lock().unwrap().as_ref() {
                        match rec.stop_with_tail(Some(tail)) {
                            Ok(capture) => capture,
                            Err(e) => {
                                error!("stop() failed: {e}");
                                Capture::default()
                            }
                        }
                    } else {
                        error!("Recorder not available");
                        Capture::default()
                    };

                *self.is_recording.lock().unwrap() = false;
                self.set_state(&mut self.state.lock().unwrap(), RecordingState::Idle);

                if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }

                // A cancelled stop still returns what it captured: the
                // pipeline's cancel check keeps it for Restore.
                Some(Capture { samples, mic_lost })
            }
            _ => None,
        }
    }

    /// The tail of a release: the debug buffer is its minimum, and the stop
    /// sound, which the microphone hears, is not taken for speech.
    pub(super) fn release_tail_request(&self, cancel_generation: u64) -> ReleaseTailRequest {
        let settings = get_settings(&self.app_handle);
        let guard = release_tail::leak_guard_ms(audio_feedback::stop_sound_length(
            &self.app_handle,
            &settings,
        ));
        let config = TailConfig::new(settings.extra_recording_buffer_ms, guard);
        debug!("Release tail: {config:?}");
        let current = Arc::clone(&self.cancel_generation);
        ReleaseTailRequest {
            config,
            abort: Arc::new(move || current.load(Ordering::Acquire) != cancel_generation),
        }
    }

    /// Cancels any ongoing recording and returns what it captured, for Restore.
    pub fn cancel_recording(&self) -> Option<Vec<f32>> {
        self.invalidate_recording_readiness();
        // An always-on or lazily closed stream stays open after a cancel, so
        // the stream teardown would not restore the user's audio.
        self.remove_mute();
        self.cancel_generation.fetch_add(1, Ordering::AcqRel);
        let mut state = self.state.lock().unwrap();

        match *state {
            RecordingState::Recording { .. } => {
                self.set_state(&mut state, RecordingState::Idle);
                drop(state);

                let captured = self
                    .recorder
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|rec| rec.stop().ok())
                    .map(|capture| capture.samples);

                *self.is_recording.lock().unwrap() = false;

                if matches!(*self.mode.lock().unwrap(), MicrophoneMode::OnDemand) {
                    if get_settings(&self.app_handle).lazy_stream_close {
                        self.schedule_lazy_close();
                    } else {
                        self.stop_microphone_stream();
                    }
                }
                captured
            }
            RecordingState::Stopping => {
                debug!("Cancellation requested while recording is stopping");
                None
            }
            RecordingState::Idle => None,
        }
    }
}
