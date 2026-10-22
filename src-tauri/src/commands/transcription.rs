use crate::codex_transcribe::CodexAuthStatus;
use crate::commands::local_models::{announce_switch_dropped, runnable_models};
use crate::engine::Engine;
use crate::settings::{get_settings, write_settings};
use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
/// Picking this Mac starts the local model's download if it is not there.
pub fn change_transcription_engine_setting(app: AppHandle, engine: Engine) -> Result<(), String> {
    let models = match engine {
        Engine::Local => Some(runnable_models(&app)?),
        Engine::Cloud => None,
    };
    let mut settings = get_settings(&app);
    let was_switching = settings.switch_to_local_when_ready;
    settings.transcription_engine = engine;
    settings.switch_to_local_when_ready = false;
    write_settings(&app, settings);
    if was_switching {
        announce_switch_dropped(&app);
    }
    if let Some(models) = models.filter(|models| !models.is_ready()) {
        models.download();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
/// The one-time card's Download: dictation moves to this Mac once the local
/// model is ready, at once if it already is.
pub fn switch_to_local_when_ready(app: AppHandle) -> Result<(), String> {
    let models = runnable_models(&app)?;
    let mut settings = get_settings(&app);
    settings.switch_to_local_when_ready = true;
    write_settings(&app, settings);
    // Asked before the check, so a download ending in between still switches.
    if models.is_ready() {
        crate::local_models::switch_if_asked(&app, true);
    } else {
        models.download();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_local_stt_card_dismissed_setting(
    app: AppHandle,
    dismissed: bool,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.local_stt_card_dismissed = dismissed;
    write_settings(&app, settings);
    Ok(())
}

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
