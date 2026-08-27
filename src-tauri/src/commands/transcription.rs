use crate::codex_transcribe::CodexAuthStatus;
use crate::settings::TranscriptionProvider;
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Type)]
pub struct GeminiStatus {
    pub available_on_platform: bool,
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
/// Reports whether Gemini transcription can use the local Antigravity install.
/// This check never starts Antigravity or reads the session token.
pub fn get_gemini_status() -> GeminiStatus {
    #[cfg(target_os = "macos")]
    {
        crate::gemini_transcribe::status()
    }

    #[cfg(not(target_os = "macos"))]
    GeminiStatus {
        available_on_platform: false,
        installed: false,
        signed_in: false,
    }
}

#[tauri::command]
#[specta::specta]
/// Opens Antigravity after an explicit user action so the user can sign in.
pub fn open_antigravity() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::gemini_transcribe::open_antigravity().map_err(|error| error.to_string())
    }

    #[cfg(not(target_os = "macos"))]
    Err("Gemini transcription is currently available on macOS only.".to_string())
}

#[tauri::command]
#[specta::specta]
/// Marks onboarding as complete in Murmur's settings store.
///
/// Keeps the selected provider when its session is usable, otherwise switches
/// to the available provider. Onboarding remains incomplete if neither works.
pub fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    let codex_signed_in = crate::codex_transcribe::auth_status().signed_in;
    #[cfg(target_os = "macos")]
    let gemini_signed_in = {
        let status = crate::gemini_transcribe::status();
        status.installed && status.signed_in
    };
    #[cfg(not(target_os = "macos"))]
    let gemini_signed_in = false;

    settings.transcription_provider = select_onboarding_provider(
        settings.transcription_provider,
        codex_signed_in,
        gemini_signed_in,
    )
    .ok_or_else(|| {
        "No usable transcription session was found. Sign in to Codex or Antigravity and retry."
            .to_string()
    })?;
    settings.onboarding_completed = true;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

fn select_onboarding_provider(
    selected: TranscriptionProvider,
    codex_signed_in: bool,
    gemini_signed_in: bool,
) -> Option<TranscriptionProvider> {
    match selected {
        TranscriptionProvider::Codex if codex_signed_in => Some(TranscriptionProvider::Codex),
        TranscriptionProvider::Gemini if gemini_signed_in => Some(TranscriptionProvider::Gemini),
        _ if codex_signed_in => Some(TranscriptionProvider::Codex),
        _ if gemini_signed_in => Some(TranscriptionProvider::Gemini),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn onboarding_selects_the_only_usable_provider() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, true),
            Some(TranscriptionProvider::Gemini)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Gemini, true, false),
            Some(TranscriptionProvider::Codex)
        );
    }

    #[test]
    fn onboarding_rejects_missing_sessions() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, false),
            None
        );
    }
}
