use crate::managers::audio::AudioRecordingManager;
use crate::shortcut;
use crate::TranscriptionCoordinator;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

pub fn cancel_current_operation(app: &AppHandle) {
    info!("Initiating operation cancellation...");

    let audio_manager = app.state::<Arc<AudioRecordingManager>>();
    let recording_was_active = audio_manager.is_recording();
    audio_manager.cancel_recording();

    // A direct recording cancellation has no processing task to clean up the
    // dynamically registered shortcut. Once processing has started, its drop
    // guard keeps this binding available until the pipeline finishes.
    if recording_was_active {
        shortcut::unregister_cancel_shortcut(app);
    }

    change_tray_icon(app, crate::tray::TrayIconState::Idle);
    hide_recording_overlay(app);

    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.notify_cancel(recording_was_active);
    }

    info!("Operation cancellation completed - returned to idle state");
}
