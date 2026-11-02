use crate::managers::audio::AudioRecordingManager;
use crate::shortcut;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub use crate::overlay::*;
pub use crate::tray::*;

/// Cancels from the coordinator thread after it has ordered the remote action.
/// The keyed coordinator ignores a stale completion, so UI and shortcut state
/// may return to idle immediately while a recorder or provider is unwinding.
pub(crate) fn cancel_current_operation_from_coordinator(app: &AppHandle) {
    cancel_current_operation_impl(app);
}

/// The user's cancel: as `cancel_current_operation_from_coordinator`, and the
/// recording's audio is kept for Restore.
pub(crate) fn cancel_and_keep_from_coordinator(app: &AppHandle) {
    // The recording being cancelled is the last dictation to have shown.
    let shown = dictation_epoch();
    if let Some(samples) = cancel_current_operation_impl(app) {
        crate::recently_cancelled::keep(app, samples, None, shown);
    }
}

/// Safe fallback for cancellation before the coordinator exists. Normal local
/// cancellation is routed through `TranscriptionCoordinator::send_cancel`.
pub(crate) fn cancel_current_operation_before_coordinator(app: &AppHandle) {
    cancel_current_operation_impl(app);
}

/// Returns the audio of the recording it cancelled, if one was recording.
fn cancel_current_operation_impl(app: &AppHandle) -> Option<Vec<f32>> {
    info!("Initiating operation cancellation...");

    let Some(audio_manager) = app.try_state::<Arc<AudioRecordingManager>>() else {
        // A public caller may run during startup, before initialize_core_logic
        // manages the audio manager. Single-instance actions are buffered by
        // SingleInstanceActionQueue; this guard only keeps other callers safe.
        log::warn!("Ignoring cancellation before the audio manager is initialized");
        return None;
    };
    let samples = audio_manager.cancel_recording();
    shortcut::unregister_cancel_shortcut(app);
    change_tray_icon(app, crate::tray::TrayIconState::Idle);
    hide_recording_overlay(app);
    info!("Recording cancellation completed - returned to idle state");
    samples
}
