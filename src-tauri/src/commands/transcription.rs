use crate::codex_transcribe::CodexAuthStatus;
use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
/// Returns whether the local Codex authentication cache contains a ChatGPT
/// session that is valid or refreshable, and whether it expired for good.
/// This check never refreshes or writes credentials.
pub fn get_codex_auth_status() -> CodexAuthStatus {
    crate::codex_transcribe::auth_status()
}

#[tauri::command]
#[specta::specta]
/// Onboarding ends only once a Codex session exists, so the first recording
/// has somewhere to go.
pub async fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    let codex_signed_in =
        tauri::async_runtime::spawn_blocking(|| crate::codex_transcribe::auth_status().signed_in)
            .await
            .map_err(|error| format!("Failed to inspect transcription configuration: {error}"))?;
    if !codex_signed_in {
        return Err("No ChatGPT/Codex session was found. Sign in to Codex and retry.".to_string());
    }
    let mut settings = crate::settings::get_settings(&app);
    settings.onboarding_completed = true;
    crate::settings::write_settings_checked(&app, settings)
}
