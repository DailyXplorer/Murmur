//! Pastes the latest successful transcription again, from its shortcut or the
//! tray. The coordinator runs it as an operation of its own, so it never
//! overlaps a dictation's paste and the cancel path still applies.

use crate::actions::ShortcutAction;
use crate::clipboard::PasteOutcome;
use crate::managers::history::{HistoryEntry, HistoryManager};
use crate::user_alert::{self, Alert};
use crate::{ProcessingOperation, TranscriptionCoordinator};
use log::{debug, error, warn};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub const BINDING_ID: &str = "paste_last";
/// Wispr Flow's default. macOS and its bundled apps leave Control+Command+V
/// unassigned, unlike Option+Shift+Command+V (paste and match style).
#[cfg(target_os = "macos")]
pub const DEFAULT_BINDING: &str = "ctrl+command+v";
/// Win+Ctrl+V opens the sound settings of Windows 11, Ctrl+Alt+V is AltGr+V
/// (a character on some layouts) and Ctrl+Shift+V pastes as plain text in
/// many apps: Alt+Shift+V is left to Murmur.
#[cfg(target_os = "windows")]
pub const DEFAULT_BINDING: &str = "alt+shift+v";

pub fn request(app: &AppHandle) {
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.send_paste_last();
    } else {
        warn!("TranscriptionCoordinator is not initialized");
    }
}

fn text_to_paste(entry: Option<HistoryEntry>) -> Option<String> {
    entry
        .map(|entry| entry.transcription_text)
        .filter(|text| !text.trim().is_empty())
}

/// Runs on the coordinator's behalf; dropping the guard ends the operation.
pub(crate) fn spawn(app: &AppHandle, operation: ProcessingOperation) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _finished = FinishGuard {
            app: app.clone(),
            operation: operation.clone(),
        };
        let latest = match app
            .state::<Arc<HistoryManager>>()
            .get_latest_completed_entry()
        {
            Ok(entry) => entry,
            Err(err) => {
                error!("Failed to read the latest transcription: {err}");
                None
            }
        };
        let Some(text) = text_to_paste(latest) else {
            debug!("No transcription to paste again");
            user_alert::show(&app, Alert::NothingToPaste);
            return;
        };
        match crate::clipboard::paste(text.clone(), app.clone(), operation).await {
            Ok(
                PasteOutcome::Injected | PasteOutcome::IntentionalNone | PasteOutcome::Cancelled,
            ) => {}
            Ok(PasteOutcome::Skipped) | Err(_) => {
                crate::clipboard::copy_for_recovery(&app, text);
                user_alert::show(&app, Alert::PasteFailed);
            }
        }
    });
}

struct FinishGuard {
    app: AppHandle,
    operation: ProcessingOperation,
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if let Some(coordinator) = self.app.try_state::<TranscriptionCoordinator>() {
            coordinator.notify_processing_finished(self.operation.id());
        }
    }
}

pub struct PasteLastAction;

impl ShortcutAction for PasteLastAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        request(app);
    }

    fn stop(
        &self,
        _app: &AppHandle,
        _binding_id: &str,
        _shortcut_str: &str,
        _operation: ProcessingOperation,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            id: 7,
            file_name: "murmur-7.wav".to_string(),
            timestamp: 0,
            saved: false,
            title: "Recording".to_string(),
            transcription_text: text.to_string(),
            raw_text: None,
            cleanup: None,
        }
    }

    #[test]
    fn pastes_the_latest_transcription_verbatim() {
        assert_eq!(
            text_to_paste(Some(entry("  Hello, world.\n"))),
            Some("  Hello, world.\n".to_string())
        );
    }

    #[test]
    fn has_nothing_to_paste_without_a_transcription() {
        assert_eq!(text_to_paste(None), None);
        assert_eq!(text_to_paste(Some(entry(" \n"))), None);
    }

    #[test]
    fn default_binding_is_a_valid_shortcut() {
        crate::shortcut::tauri_impl::parse_shortcut(DEFAULT_BINDING).unwrap();
        assert!(DEFAULT_BINDING
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok());
    }
}
