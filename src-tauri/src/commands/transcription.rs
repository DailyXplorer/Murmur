use crate::codex_transcribe::CodexAuthStatus;

#[tauri::command]
#[specta::specta]
pub fn get_codex_auth_status() -> CodexAuthStatus {
    crate::codex_transcribe::auth_status()
}
