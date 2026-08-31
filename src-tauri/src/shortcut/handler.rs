//! Shared shortcut event handling logic.

use log::warn;
use tauri::{AppHandle, Manager};

use crate::actions::ACTION_MAP;
use crate::settings::get_settings;
use crate::transcription_coordinator::is_transcribe_binding;
use crate::TranscriptionCoordinator;

fn should_dispatch_cancel(is_pressed: bool) -> bool {
    is_pressed
}

/// Handle a shortcut event from the global-shortcut implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding during recording or processing
/// - Handling push-to-talk mode (start on press, stop on release)
/// - Handling toggle mode (toggle state on press only)
///
/// # Arguments
/// * `app` - The Tauri app handle
/// * `binding_id` - The ID of the binding (e.g., "transcribe", "cancel")
/// * `hotkey_string` - The string representation of the hotkey
/// * `is_pressed` - Whether this is a key press (true) or release (false)
pub fn handle_shortcut_event(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    let settings = get_settings(app);

    // Transcribe bindings are handled by the coordinator.
    if is_transcribe_binding(binding_id) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.send_input(binding_id, hotkey_string, is_pressed, settings.push_to_talk);
        } else {
            warn!("TranscriptionCoordinator is not initialized");
        }
        return;
    }

    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!(
            "No action defined in ACTION_MAP for shortcut ID '{}'. Shortcut: '{}', Pressed: {}",
            binding_id, hotkey_string, is_pressed
        );
        return;
    };

    // The binding stays registered until the pipeline's finish guard runs, so
    // a press must reach cancellation while WAV saving or transcription runs.
    if binding_id == "cancel" {
        if should_dispatch_cancel(is_pressed) {
            action.start(app, binding_id, hotkey_string);
        }
        return;
    }

    // Remaining bindings (e.g. "test") use simple start/stop on press/release.
    if is_pressed {
        action.start(app, binding_id, hotkey_string);
    } else {
        action.stop(app, binding_id, hotkey_string);
    }
}

#[cfg(test)]
mod tests {
    use super::should_dispatch_cancel;

    #[test]
    fn cancel_press_is_dispatched_without_a_recording_state_check() {
        assert!(should_dispatch_cancel(true));
    }

    #[test]
    fn cancel_release_is_not_dispatched() {
        assert!(!should_dispatch_cancel(false));
    }
}
