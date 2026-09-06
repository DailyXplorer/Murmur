use crate::codex_transcribe::CodexAuthStatus;
use crate::settings::TranscriptionProvider;
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Type)]
pub struct GeminiStatus {
    pub installed: bool,
    pub signed_in: bool,
}

#[tauri::command]
#[specta::specta]
/// Returns whether the local Codex authentication cache contains a usable
/// ChatGPT session. This check never refreshes or writes credentials.
pub fn get_codex_auth_status() -> CodexAuthStatus {
    crate::codex_transcribe::auth_status()
}

#[tauri::command]
#[specta::specta]
/// Signature verification runs off the AppKit thread. This only reports local
/// configuration, not whether the cloud will accept a transcription.
pub async fn get_gemini_status() -> GeminiStatus {
    tauri::async_runtime::spawn_blocking(crate::gemini_transcribe::status)
        .await
        .unwrap_or_else(|error| {
            log::error!("Failed to inspect Antigravity: {error}");
            GeminiStatus {
                installed: false,
                signed_in: false,
            }
        })
}

#[tauri::command]
#[specta::specta]
/// Opens Antigravity after an explicit user action so the user can sign in.
pub fn open_antigravity() -> Result<(), String> {
    crate::gemini_transcribe::open_antigravity().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
/// Marks onboarding as complete in Murmur's settings store.
///
/// The provider shown by onboarding must remain selected. Losing its local
/// configuration cannot silently change where the next recording is sent.
pub async fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    let (codex_signed_in, gemini_signed_in) = tauri::async_runtime::spawn_blocking(|| {
        let codex_signed_in = crate::codex_transcribe::auth_status().signed_in;
        let status = crate::gemini_transcribe::status();
        (codex_signed_in, status.installed && status.signed_in)
    })
    .await
    .map_err(|error| format!("Failed to inspect transcription configuration: {error}"))?;
    let mut settings = crate::settings::get_settings(&app);
    settings.transcription_provider = select_onboarding_provider(
        settings.transcription_provider,
        codex_signed_in,
        gemini_signed_in,
    )
    .ok_or_else(|| {
        "The selected transcription service is not configured. Sign in or choose another service and retry."
            .to_string()
    })?;
    settings.onboarding_completed = true;
    crate::settings::write_settings_checked(&app, settings)
}

/// Never substitutes another cloud destination for the user's chosen provider.
fn select_onboarding_provider(
    selected: TranscriptionProvider,
    codex_signed_in: bool,
    gemini_signed_in: bool,
) -> Option<TranscriptionProvider> {
    match selected {
        TranscriptionProvider::Codex if codex_signed_in => Some(TranscriptionProvider::Codex),
        TranscriptionProvider::Gemini if gemini_signed_in => Some(TranscriptionProvider::Gemini),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps Codex or Gemini when that provider already has a usable session.
    #[test]
    fn onboarding_keeps_a_usable_selected_provider() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, true, true),
            Some(TranscriptionProvider::Codex)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Gemini, true, true),
            Some(TranscriptionProvider::Gemini)
        );
    }

    #[test]
    fn onboarding_does_not_replace_an_unavailable_selected_provider() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, true),
            None
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Gemini, true, false),
            None
        );
    }

    /// Rejects onboarding when neither Codex nor Gemini has a usable session.
    #[test]
    fn onboarding_rejects_missing_sessions() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, false),
            None
        );
    }
}
