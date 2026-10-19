use crate::TranscriptionCoordinator;
use log::warn;
use tauri::{AppHandle, Manager};

/// Send a transcription input to the coordinator.
/// Used by signal handlers, CLI flags, and any other external trigger.
pub fn send_transcription_input(app: &AppHandle, binding_id: &str, source: &str) {
    if let Some(c) = app.try_state::<TranscriptionCoordinator>() {
        c.send_toggle(binding_id, source);
    } else {
        warn!("TranscriptionCoordinator not initialized");
    }
}

/// Listen for SIGUSR2 to remotely toggle transcription. Windows has no
/// signals; `--toggle-transcription` does the same there.
#[cfg(unix)]
pub fn setup_signal_handler(app_handle: AppHandle) {
    use log::debug;
    use signal_hook::consts::SIGUSR2;
    use signal_hook::iterator::Signals;
    use std::thread;

    let mut signals =
        Signals::new([SIGUSR2]).expect("failed to register transcription signal handlers");
    debug!("Signal handler registered for SIGUSR2");
    thread::spawn(move || {
        for sig in signals.forever() {
            let (binding_id, signal_name) = match sig {
                SIGUSR2 => ("transcribe", "SIGUSR2"),
                _ => continue,
            };
            debug!("Received {signal_name}");
            send_transcription_input(&app_handle, binding_id, signal_name);
        }
    });
}
