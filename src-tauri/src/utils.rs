use crate::managers::audio::AudioRecordingManager;
use crate::shortcut;
use log::info;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub use crate::clipboard::*;
pub use crate::overlay::*;
pub use crate::tray::*;

/// The pipeline state supplied by the coordinator when it authorizes a
/// cancellation. Audio state is deliberately not used to classify this: while
/// the stop worker is running, it can briefly remain active during Processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancellationStage {
    NotProcessing,
    Processing,
}

/// Side effects permitted for a cancellation in a given authoritative stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CancellationEffects {
    pub(crate) signal_audio_cancellation: bool,
    pub(crate) unregister_cancel_shortcut: bool,
    pub(crate) set_tray_idle: bool,
    pub(crate) hide_recording_overlay: bool,
}

pub(crate) fn cancellation_effects(
    stage: CancellationStage,
    audio_is_active: bool,
) -> CancellationEffects {
    let clean_up_immediately = audio_is_active && matches!(stage, CancellationStage::NotProcessing);

    CancellationEffects {
        signal_audio_cancellation: true,
        unregister_cancel_shortcut: clean_up_immediately,
        set_tray_idle: clean_up_immediately,
        hide_recording_overlay: clean_up_immediately,
    }
}

/// Cancels from the coordinator thread after it has ordered the remote action.
/// `stage` is the coordinator's authoritative state, which keeps the UI
/// truthful while the stop task releases the audio manager asynchronously.
pub(crate) fn cancel_current_operation_from_coordinator(app: &AppHandle, stage: CancellationStage) {
    cancel_current_operation_impl(app, stage)
}

/// Safe fallback for cancellation before the coordinator exists. Normal local
/// cancellation is routed through `TranscriptionCoordinator::send_cancel`.
pub(crate) fn cancel_current_operation_before_coordinator(app: &AppHandle) {
    cancel_current_operation_impl(app, CancellationStage::NotProcessing)
}

fn cancel_current_operation_impl(app: &AppHandle, stage: CancellationStage) {
    info!("Initiating operation cancellation...");

    let Some(audio_manager) = app.try_state::<Arc<AudioRecordingManager>>() else {
        // A public caller may run during startup, before initialize_core_logic
        // manages the audio manager. Single-instance actions are buffered by
        // SingleInstanceActionQueue; this guard only keeps other callers safe.
        log::warn!("Ignoring cancellation before the audio manager is initialized");
        return;
    };
    let recording_was_active = audio_manager.is_recording();
    let effects = cancellation_effects(stage, recording_was_active);

    if effects.signal_audio_cancellation {
        audio_manager.cancel_recording();
    }

    // A direct recording cancellation has no processing task to clean up the
    // dynamically registered shortcut. During processing the provider worker
    // cannot be interrupted, so keep the overlay and tray in their truthful
    // processing state until the pipeline observes this cancellation.
    if effects.unregister_cancel_shortcut {
        shortcut::unregister_cancel_shortcut(app);
    }
    if effects.set_tray_idle {
        change_tray_icon(app, crate::tray::TrayIconState::Idle);
    }
    if effects.hide_recording_overlay {
        hide_recording_overlay(app);
    }
    if effects.unregister_cancel_shortcut {
        info!("Recording cancellation completed - returned to idle state");
    } else {
        info!("Processing cancellation requested - waiting for worker completion");
    }
}

#[cfg(test)]
mod tests {
    use super::{cancellation_effects, CancellationEffects, CancellationStage};

    #[test]
    fn active_audio_during_processing_only_signals_cancellation() {
        assert_eq!(
            cancellation_effects(CancellationStage::Processing, true),
            CancellationEffects {
                signal_audio_cancellation: true,
                unregister_cancel_shortcut: false,
                set_tray_idle: false,
                hide_recording_overlay: false,
            }
        );
    }

    #[test]
    fn active_audio_during_recording_cleans_up_immediately() {
        assert_eq!(
            cancellation_effects(CancellationStage::NotProcessing, true),
            CancellationEffects {
                signal_audio_cancellation: true,
                unregister_cancel_shortcut: true,
                set_tray_idle: true,
                hide_recording_overlay: true,
            }
        );
    }
}
