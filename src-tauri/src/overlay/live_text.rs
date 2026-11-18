//! The words of a dictation next to its orb, while ChatGPT hears them
//! (`dictation_live_text`).

use super::*;
use crate::codex_stream::Listener;
use crate::engine::Engine;
use crate::settings::AppSettings;

/// Whether a dictation shows its words: the setting is on, there is an
/// overlay to show them in, and ChatGPT is the engine the user chose. With
/// the local engine the setting's switch reads off, so a dictation that
/// falls back to ChatGPT shows no words either.
pub(super) fn shown(settings: &AppSettings) -> bool {
    settings.dictation_live_text
        && settings.overlay_style != OverlayStyle::None
        && settings.transcription_engine == Engine::Cloud
}

/// The window of both dictation states. With the words it has the width of
/// the widest capsule from the start; the orb sits in its middle, where the
/// window of the orb alone has it, and the window ignores the mouse.
pub(super) fn dictation_window_size(settings: &AppSettings) -> (f64, f64) {
    if shown(settings) {
        (LIVE_TEXT_OVERLAY_WIDTH, ORB_OVERLAY_SIZE)
    } else {
        (ORB_OVERLAY_SIZE, ORB_OVERLAY_SIZE)
    }
}

/// The recording whose dictation a state shows, given the one before it. A
/// recording starts a dictation at its own epoch; the transcribing state
/// after it is the same dictation.
pub(super) fn recording_shown(
    state: &str,
    epoch: DictationEpoch,
    recording: DictationEpoch,
) -> DictationEpoch {
    if state == "recording" {
        epoch
    } else {
        recording
    }
}

/// Whether text heard for the dictation whose recording showed at `opened`
/// still has that dictation on screen: a dictation shows, and no later
/// recording took the overlay.
pub(super) fn reaches(
    opened: DictationEpoch,
    recording: DictationEpoch,
    dictation_showing: bool,
) -> bool {
    dictation_showing && opened == recording
}

/// What hears the stream of the dictation that records now, when its words
/// are to show. Each text is checked and emitted on the main thread, where
/// the overlay's states are emitted too: a text never lands in the page
/// after the state of a newer dictation, or after the hide.
pub(crate) fn listener(app_handle: &AppHandle, settings: &AppSettings) -> Option<Listener> {
    if !shown(settings) {
        return None;
    }
    let opened = recording_epoch();
    let app = app_handle.clone();
    Some(Box::new(move |text: &str| {
        let text = text.to_string();
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let showing = DICTATION_SHOWING.load(Ordering::SeqCst);
            if reaches(opened, recording_epoch(), showing) {
                let _ = handle.emit_to("recording_overlay", "dictation-live-text", text);
            }
        });
    }))
}
