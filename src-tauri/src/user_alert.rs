//! Tells the user about a problem where they are looking. Dictation failures
//! go to the recording overlay; when the overlay is off, and for problems
//! outside a dictation, a macOS notification is posted instead. The settings
//! window keeps its own toasts.

use crate::meetings::coordinator::MeetingAlert;
use crate::settings::{get_settings, SettingsRecovery};
use crate::transcription_error::FailureKind;
use crate::tray_i18n::{get_tray_translations, TrayStrings};
use chrono::TimeZone;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// A dictation failed to transcribe.
    Transcription(FailureKind),
    PasteFailed,
    RecordingFailed,
    MicrophoneDenied,
    LongRecording,
    RecordingLimitSoon,
    NothingToPaste,
    ShortcutUnavailable {
        shortcut: String,
    },
    MicrophoneUnavailableAtStartup,
    HistoryUnavailable,
    HistoryReset,
    SettingsRecovered(SettingsRecovery),
    Meeting(MeetingAlert),
    /// At launch: meetings a crash cut off are being processed.
    MeetingsRecovered {
        count: usize,
    },
    /// At launch: the meetings library was damaged or deleted, and its
    /// meetings were set aside.
    MeetingsReset,
}

impl Alert {
    /// Failures of the dictation in progress belong in the overlay.
    fn belongs_in_overlay(&self) -> bool {
        matches!(
            self,
            Self::Transcription(..)
                | Self::PasteFailed
                | Self::RecordingFailed
                | Self::MicrophoneDenied
        )
    }

    /// The text in `language`, whose `strings` the caller looked up.
    fn message(&self, strings: &TrayStrings, language: &str) -> String {
        match self {
            Self::Transcription(kind) => match kind {
                FailureKind::Network => &strings.error_network,
                FailureKind::Timeout => &strings.error_timeout,
                FailureKind::RateLimited => &strings.error_rate_limited,
                FailureKind::ServerError | FailureKind::ConnectionDropped => &strings.error_server,
                FailureKind::SessionExpired => &strings.error_session_expired,
                FailureKind::SignedOut => &strings.error_signed_out,
                FailureKind::TooLarge => &strings.error_too_large,
                FailureKind::LocalNotReady => &strings.error_local_not_ready,
                FailureKind::LocalFailed => &strings.error_local_failed,
                FailureKind::LanguageNeedsCloud => &strings.error_language_needs_cloud,
                FailureKind::Rejected | FailureKind::Other => &strings.error_transcription,
            }
            .clone(),
            Self::PasteFailed => strings.error_paste.clone(),
            Self::RecordingFailed => strings.error_recording.clone(),
            Self::MicrophoneDenied if cfg!(target_os = "windows") => {
                strings.error_microphone_denied_windows.clone()
            }
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
            Self::SettingsRecovered(SettingsRecovery::Restored) => {
                strings.notice_settings_restored.clone()
            }
            Self::SettingsRecovered(SettingsRecovery::Reset) => {
                strings.notice_settings_reset.clone()
            }
            Self::Meeting(alert) => match alert {
                MeetingAlert::SystemAudioUnavailable => &strings.meeting_system_audio_unavailable,
                MeetingAlert::MicrophoneUnavailable => &strings.meeting_microphone_unavailable,
                MeetingAlert::StartFailed => &strings.meeting_start_failed,
                MeetingAlert::StopsSoon => &strings.meeting_stops_soon,
                MeetingAlert::StoppedAtLimit => &strings.meeting_stopped_at_limit,
                MeetingAlert::WriteFailed => &strings.meeting_write_failed,
                MeetingAlert::LowDiskSpace => &strings.meeting_low_disk_space,
                MeetingAlert::SystemAudioInterrupted => &strings.meeting_system_audio_interrupted,
                MeetingAlert::MicrophoneInterrupted => &strings.meeting_microphone_interrupted,
                MeetingAlert::MicrophoneDenied => &strings.meeting_microphone_denied,
                MeetingAlert::SystemAudioDenied => &strings.meeting_system_audio_denied,
                MeetingAlert::AudioDropped => &strings.meeting_audio_dropped,
                MeetingAlert::PausedWhileAsleep { from_ms, to_ms } => {
                    return strings
                        .meeting_paused_while_asleep
                        .replace("{{from}}", &clock_time(*from_ms, language))
                        .replace("{{to}}", &clock_time(*to_ms, language))
                }
            }
            .clone(),
            Self::MeetingsRecovered { count: 1 } => strings.meeting_recovered.clone(),
            Self::MeetingsRecovered { count } => strings
                .meetings_recovered
                .replace("{{count}}", &count.to_string()),
            Self::MeetingsReset => strings.meetings_reset.clone(),
        }
    }
}

/// Windows names its modifier keys Ctrl, Alt and Win.
#[cfg(target_os = "windows")]
fn windows_key_name(part: &str) -> Option<&'static str> {
    match part.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some("Ctrl"),
        "alt" | "option" => Some("Alt"),
        "super" | "command" | "cmd" | "meta" | "win" | "windows" => Some("Win"),
        _ => None,
    }
}

/// The time of day on this Mac: "3:42 PM" in English, "15:42" otherwise.
fn clock_time(unix_ms: i64, language: &str) -> String {
    let Some(time) = chrono::Local.timestamp_millis_opt(unix_ms).single() else {
        return String::new();
    };
    let english = language.to_lowercase().starts_with("en");
    time.format(if english { "%-I:%M %p" } else { "%H:%M" })
        .to_string()
}

/// "option+space" becomes "Option+Space", and "Alt+Space" on Windows.
fn display_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| {
            #[cfg(target_os = "windows")]
            if let Some(name) = windows_key_name(part.trim()) {
                return name.to_string();
            }
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
    let language = get_settings(app).app_language;
    let strings = get_tray_translations(Some(language.clone()));
    let message = alert.message(&strings, &language);
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
            FailureKind::LocalNotReady,
            FailureKind::LocalFailed,
            FailureKind::LanguageNeedsCloud,
            FailureKind::Other,
        ];
        let messages: std::collections::HashSet<_> = kinds
            .iter()
            .map(|kind| Alert::Transcription(*kind).message(&strings, "en"))
            .collect();
        assert_eq!(messages.len(), kinds.len());
        assert!(messages.iter().all(|message| !message.is_empty()));
    }

    #[test]
    fn every_meeting_alert_has_its_own_message_in_a_notification() {
        let strings = get_tray_translations(Some("fr".to_string()));
        let alerts = [
            MeetingAlert::SystemAudioUnavailable,
            MeetingAlert::MicrophoneUnavailable,
            MeetingAlert::StartFailed,
            MeetingAlert::StopsSoon,
            MeetingAlert::StoppedAtLimit,
            MeetingAlert::WriteFailed,
            MeetingAlert::LowDiskSpace,
            MeetingAlert::SystemAudioInterrupted,
            MeetingAlert::MicrophoneInterrupted,
            MeetingAlert::MicrophoneDenied,
            MeetingAlert::SystemAudioDenied,
            MeetingAlert::PausedWhileAsleep {
                from_ms: 1_760_000_000_000,
                to_ms: 1_760_001_200_000,
            },
            MeetingAlert::AudioDropped,
        ];
        let messages: std::collections::HashSet<_> = alerts
            .iter()
            .map(|alert| Alert::Meeting(*alert).message(&strings, "fr"))
            .collect();
        assert_eq!(messages.len(), alerts.len());
        assert!(alerts
            .iter()
            .all(|alert| !Alert::Meeting(*alert).belongs_in_overlay()));
    }

    #[test]
    fn recovered_meetings_are_told_in_one_notification_with_their_count() {
        for language in ["en", "fr", "es", "de", "it", "pt"] {
            let strings = get_tray_translations(Some(language.to_string()));
            let one = Alert::MeetingsRecovered { count: 1 }.message(&strings, language);
            let three = Alert::MeetingsRecovered { count: 3 }.message(&strings, language);
            assert!(!one.is_empty() && !one.contains("{{"), "{language}: {one}");
            assert!(
                three.contains('3') && !three.contains("{{"),
                "{language}: {three}"
            );
            assert_ne!(one, three);
            assert!(!Alert::MeetingsRecovered { count: 1 }.belongs_in_overlay());
        }
    }

    #[test]
    fn a_sleep_during_a_meeting_is_told_with_the_times_nothing_was_recorded() {
        let from = chrono::Local
            .with_ymd_and_hms(2026, 10, 22, 14, 5, 0)
            .unwrap()
            .timestamp_millis();
        let alert = Alert::Meeting(MeetingAlert::PausedWhileAsleep {
            from_ms: from,
            to_ms: from + 20 * 60_000,
        });
        assert_eq!(
            alert.message(&get_tray_translations(Some("en".to_string())), "en"),
            "Your Mac slept during the meeting. Nothing was recorded from 2:05 PM to 2:25 PM."
        );
        let french = alert.message(&get_tray_translations(Some("fr".to_string())), "fr");
        assert!(french.contains("de 14:05 à 14:25"), "{french}");
        for language in ["es", "de", "it", "pt"] {
            let message =
                alert.message(&get_tray_translations(Some(language.to_string())), language);
            assert!(
                message.contains("14:05") && message.contains("14:25") && !message.contains("{{"),
                "{language}: {message}"
            );
        }
    }

    #[test]
    fn shortcut_message_names_the_shortcut() {
        let strings = get_tray_translations(Some("en".to_string()));
        let message = Alert::ShortcutUnavailable {
            shortcut: "option+space".to_string(),
        }
        .message(&strings, "en");
        let expected = if cfg!(target_os = "windows") {
            "Alt+Space"
        } else {
            "Option+Space"
        };
        assert!(message.contains(expected), "{message}");
        assert!(!message.contains("{{"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_names_the_default_shortcut_with_its_own_keys() {
        assert_eq!(display_shortcut("ctrl+super"), "Ctrl+Win");
        assert_eq!(display_shortcut("alt+shift+v"), "Alt+Shift+V");
    }
}
