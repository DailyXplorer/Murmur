//! "Paste new meeting text": pastes the live sentences published since the
//! last use in this meeting, or those of the last minute at the first one,
//! as `You: …` / `Others: …` lines. Run by the coordinator as an operation of
//! its own, like pasting the last transcription again.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use super::{paste_text, Live};
use crate::actions::ShortcutAction;
use crate::clipboard::PasteOutcome;
use crate::settings::get_settings;
use crate::tray_i18n::get_tray_translations;
use crate::user_alert::{self, Alert};
use crate::{ProcessingOperation, TranscriptionCoordinator};

pub(crate) fn spawn(app: &AppHandle, operation: ProcessingOperation) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _finished = Finished {
            app: app.clone(),
            operation: operation.clone(),
        };
        let entries = app
            .try_state::<Arc<Live>>()
            .map(|live| live.take_new())
            .unwrap_or_default();
        if entries.is_empty() {
            user_alert::show(&app, Alert::NothingNewInMeeting);
            return;
        }
        let strings = get_tray_translations(Some(get_settings(&app).app_language));
        let text = paste_text(&entries, &strings.live_you, &strings.live_others);
        match crate::clipboard::paste(text.clone(), app.clone(), operation, false).await {
            Ok(
                PasteOutcome::Injected
                | PasteOutcome::CopyOffered
                | PasteOutcome::IntentionalNone
                | PasteOutcome::Cancelled,
            ) => {}
            Ok(PasteOutcome::Skipped) | Err(_) => {
                crate::clipboard::copy_for_recovery(&app, text);
                user_alert::show(&app, Alert::PasteFailed);
            }
        }
    });
}

struct Finished {
    app: AppHandle,
    operation: ProcessingOperation,
}

impl Drop for Finished {
    fn drop(&mut self) {
        if let Some(coordinator) = self.app.try_state::<TranscriptionCoordinator>() {
            coordinator.notify_processing_finished(self.operation.id());
        }
    }
}

pub struct PasteNewAction;

impl ShortcutAction for PasteNewAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.send_paste_meeting();
        }
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
