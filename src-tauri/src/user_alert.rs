//! Tells the user about a problem where they are looking. Dictation failures
//! go to the recording overlay; when the overlay is off, and for problems
//! outside a dictation, a macOS notification is posted instead. The settings
//! window keeps its own toasts.

use crate::settings::get_settings;
use crate::transcription_error::FailureKind;
use crate::tray_i18n::{get_tray_translations, TrayStrings};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    Transcription(FailureKind),
    PasteFailed,
    RecordingFailed,
    MicrophoneDenied,
    LongRecording,
    RecordingLimitSoon,
    NothingToPaste,
    ShortcutUnavailable { shortcut: String },
    MicrophoneUnavailableAtStartup,
    HistoryUnavailable,
    HistoryReset,
}

impl Alert {
    /// Failures of the dictation in progress belong in the overlay.
    fn belongs_in_overlay(&self) -> bool {
        matches!(
            self,
            Self::Transcription(_)
                | Self::PasteFailed
                | Self::RecordingFailed
                | Self::MicrophoneDenied
        )
    }

    fn message(&self, strings: &TrayStrings) -> String {
        match self {
            Self::Transcription(kind) => match kind {
                FailureKind::Network => &strings.error_network,
                FailureKind::Timeout => &strings.error_timeout,
                FailureKind::RateLimited => &strings.error_rate_limited,
                FailureKind::ServerError => &strings.error_server,
                FailureKind::SessionExpired => &strings.error_session_expired,
                FailureKind::SignedOut => &strings.error_signed_out,
                FailureKind::TooLarge => &strings.error_too_large,
                FailureKind::Rejected | FailureKind::Other => &strings.error_transcription,
            }
            .clone(),
            Self::PasteFailed => strings.error_paste.clone(),
            Self::RecordingFailed => strings.error_recording.clone(),
            Self::MicrophoneDenied => strings.error_microphone_denied.clone(),
            Self::LongRecording => strings.warning_long_recording.clone(),
            Self::RecordingLimitSoon => strings.warning_recording_limit.clone(),
            Self::NothingToPaste => strings.notice_nothing_to_paste.clone(),
            Self::ShortcutUnavailable { shortcut } => strings
                .error_shortcut_unavailable
                .replace("{{shortcut}}", &display_shortcut(shortcut)),
            Self::MicrophoneUnavailableAtStartup => strings.error_microphone_startup.clone(),
            Self::HistoryUnavailable => strings.error_history_startup.clone(),
            Self::HistoryReset => strings.notice_history_reset.clone(),
        }
    }
}

/// "option+space" becomes "Option+Space".
fn display_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| {
            let mut characters = part.trim().chars();
            characters
                .next()
                .map(|first| first.to_uppercase().chain(characters).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join("+")
}

pub fn show(app: &AppHandle, alert: Alert) {
    let strings = get_tray_translations(Some(get_settings(app).app_language));
    let message = alert.message(&strings);
    if alert.belongs_in_overlay() && crate::overlay::show_error_overlay(app, message.clone()) {
        return;
    }
    if let Err(error) = app
        .notification()
        .builder()
        .title("Murmur")
        .body(message)
        .show()
    {
        log::error!("Failed to post a notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_kind_has_its_own_message() {
        let strings = get_tray_translations(Some("en".to_string()));
        let kinds = [
            FailureKind::Network,
            FailureKind::Timeout,
            FailureKind::RateLimited,
            FailureKind::ServerError,
            FailureKind::SessionExpired,
            FailureKind::SignedOut,
            FailureKind::TooLarge,
            FailureKind::Other,
        ];
        let messages: std::collections::HashSet<_> = kinds
            .iter()
            .map(|kind| Alert::Transcription(*kind).message(&strings))
            .collect();
        assert_eq!(messages.len(), kinds.len());
        assert!(messages.iter().all(|message| !message.is_empty()));
    }

    #[test]
    fn shortcut_message_names_the_shortcut() {
        let strings = get_tray_translations(Some("en".to_string()));
        let message = Alert::ShortcutUnavailable {
            shortcut: "option+space".to_string(),
        }
        .message(&strings);
        assert!(message.contains("Option+Space"), "{message}");
        assert!(!message.contains("{{"));
    }
}
