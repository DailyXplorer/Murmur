use crate::codex_transcribe::CodexAuthStatus;
use crate::meta_app::MetaAppStatus;
use crate::meta_transcribe::MetaApiStatus;
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
/// Reports whether Gemini transcription can use the local Antigravity install.
/// This check never starts Antigravity or reads the session token.
pub fn get_gemini_status() -> GeminiStatus {
    crate::gemini_transcribe::status()
}

#[tauri::command]
#[specta::specta]
pub fn get_meta_api_status() -> MetaApiStatus {
    crate::meta_transcribe::status()
}

#[tauri::command]
#[specta::specta]
/// Reports whether Meta AI for Mac can provide background dictation. This
/// never reads Meta credentials or starts the application.
pub fn get_meta_app_status() -> MetaAppStatus {
    crate::meta_app::status()
}

#[tauri::command]
#[specta::specta]
pub fn save_meta_api_key(api_key: String) -> Result<(), String> {
    crate::meta_transcribe::save_api_key(&api_key).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn clear_meta_api_key(app: AppHandle) -> Result<(), String> {
    if crate::settings::get_settings(&app).transcription_provider == TranscriptionProvider::Meta {
        return Err(
            "Select another transcription service before removing the active Meta API key."
                .to_string(),
        );
    }
    crate::meta_transcribe::clear_api_key().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
/// Opens Antigravity after an explicit user action so the user can sign in.
pub fn open_antigravity() -> Result<(), String> {
    crate::gemini_transcribe::open_antigravity().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
/// Opens Meta AI after an explicit user action so dictation can be configured.
pub fn open_meta_ai() -> Result<(), String> {
    crate::meta_app::open_meta_ai().map_err(|error| error.to_string())
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
    let gemini_signed_in = {
        let status = crate::gemini_transcribe::status();
        status.installed && status.signed_in
    };
    let meta_configured = crate::meta_transcribe::status().configured;
    let meta_app_ready = crate::meta_app::status().ready;
    settings.transcription_provider = select_onboarding_provider(
        settings.transcription_provider,
        codex_signed_in,
        gemini_signed_in,
        meta_configured,
        meta_app_ready,
    )
    .ok_or_else(|| {
        "No usable transcription service was found. Sign in to Codex or Antigravity, configure Meta AI dictation, or add a Meta Model API key, and retry."
            .to_string()
    })?;
    settings.onboarding_completed = true;
    crate::settings::write_settings(&app, settings);
    Ok(())
}

/// Keeps `selected` when that provider has a usable session, otherwise prefers
fn select_onboarding_provider(
    selected: TranscriptionProvider,
    codex_signed_in: bool,
    gemini_signed_in: bool,
    meta_configured: bool,
    meta_app_ready: bool,
) -> Option<TranscriptionProvider> {
    match selected {
        TranscriptionProvider::Codex if codex_signed_in => Some(TranscriptionProvider::Codex),
        TranscriptionProvider::Gemini if gemini_signed_in => Some(TranscriptionProvider::Gemini),
        TranscriptionProvider::Meta if meta_configured => Some(TranscriptionProvider::Meta),
        TranscriptionProvider::MetaApp if meta_app_ready => Some(TranscriptionProvider::MetaApp),
        _ if codex_signed_in => Some(TranscriptionProvider::Codex),
        _ if gemini_signed_in => Some(TranscriptionProvider::Gemini),
        _ if meta_configured => Some(TranscriptionProvider::Meta),
        _ if meta_app_ready => Some(TranscriptionProvider::MetaApp),
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
            select_onboarding_provider(TranscriptionProvider::Codex, true, true, true, true),
            Some(TranscriptionProvider::Codex)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Gemini, true, true, true, true),
            Some(TranscriptionProvider::Gemini)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Meta, true, true, true, true),
            Some(TranscriptionProvider::Meta)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::MetaApp, true, true, true, true,),
            Some(TranscriptionProvider::MetaApp)
        );
    }

    /// Falls back to the other provider when the selected session is missing.
    #[test]
    fn onboarding_selects_the_only_usable_provider() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, true, false, false),
            Some(TranscriptionProvider::Gemini)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Gemini, true, false, false, false),
            Some(TranscriptionProvider::Codex)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, false, true, false),
            Some(TranscriptionProvider::Meta)
        );
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, false, false, true,),
            Some(TranscriptionProvider::MetaApp)
        );
    }

    /// Rejects onboarding when neither Codex nor Gemini has a usable session.
    #[test]
    fn onboarding_rejects_missing_sessions() {
        assert_eq!(
            select_onboarding_provider(TranscriptionProvider::Codex, false, false, false, false,),
            None
        );
    }
}
