use crate::codex_transcribe::CodexAuthStatus;
use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
/// Returns whether the local Codex authentication cache contains a usable
/// ChatGPT session. This check never refreshes or writes credentials.
pub fn get_codex_auth_status() -> CodexAuthStatus {
    crate::codex_transcribe::auth_status()
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
