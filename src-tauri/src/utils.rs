use crate::managers::audio::AudioRecordingManager;
use crate::shortcut;
use crate::TranscriptionCoordinator;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// Cancels the active recording or requests cancellation for processing.
pub fn cancel_current_operation(app: &AppHandle) {
    cancel_current_operation_impl(app, true, false)
}

/// Cancels from the coordinator thread after it has ordered the remote action.
/// The coordinator updates its own state synchronously, so it must not enqueue
/// a second cancellation that could race a following remote toggle. `processing`
/// keeps the UI in its truthful processing state while the stop task releases
/// the audio manager asynchronously.
pub(crate) fn cancel_current_operation_from_coordinator(app: &AppHandle, processing: bool) {
    cancel_current_operation_impl(app, false, processing)
}

fn cancel_current_operation_impl(app: &AppHandle, notify_coordinator: bool, processing: bool) {
    info!("Initiating operation cancellation...");

    let Some(audio_manager) = app.try_state::<Arc<AudioRecordingManager>>() else {
        // A public caller may run during startup, before initialize_core_logic
        // manages the audio manager. Single-instance actions are buffered by
        // SingleInstanceActionQueue; this guard only keeps other callers safe.
        log::warn!("Ignoring cancellation before the audio manager is initialized");
        return;
    };
    let recording_was_active = audio_manager.is_recording();

    audio_manager.cancel_recording();

    // A direct recording cancellation has no processing task to clean up the
    // dynamically registered shortcut. During processing the provider worker
    // cannot be interrupted, so keep the overlay and tray in their truthful
    // processing state until the pipeline observes this cancellation.
    if recording_was_active && !processing {
        shortcut::unregister_cancel_shortcut(app);
        change_tray_icon(app, crate::tray::TrayIconState::Idle);
        hide_recording_overlay(app);
        info!("Recording cancellation completed - returned to idle state");
    } else {
        info!("Processing cancellation requested - waiting for worker completion");
    }

    if notify_coordinator {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.notify_cancel(recording_was_active);
        }
    }
}
