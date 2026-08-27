use crate::codex_transcribe::CodexAuthStatus;
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
/// Returns an error string only if the command contract changes to expose a
/// persistence failure; the current store API completes synchronously.
pub fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.onboarding_completed = true;
    crate::settings::write_settings(&app, settings);
    Ok(())
}
