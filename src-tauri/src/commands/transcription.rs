use crate::codex_transcribe::CodexAuthStatus;
use crate::commands::local_models::{announce_switch_dropped, runnable_models};
use crate::engine::Engine;
use crate::settings::{update_settings, AppSettings};
use tauri::AppHandle;

#[tauri::command]
#[specta::specta]
/// Picking this Mac starts the local model's download if it is not there.
pub fn change_transcription_engine_setting(app: AppHandle, engine: Engine) -> Result<(), String> {
    let models = match engine {
        Engine::Local => Some(runnable_models(&app)?),
        Engine::Cloud => None,
    };
    let was_switching = update_settings(&app, |settings| {
        let was_switching = settings.switch_to_local_when_ready;
        settings.transcription_engine = engine;
        settings.switch_to_local_when_ready = false;
        was_switching
    });
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
    update_settings(&app, |settings| settings.switch_to_local_when_ready = true);
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
    update_settings(&app, |settings| {
        settings.local_stt_card_dismissed = dismissed
    });
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

/// Ends onboarding on the engine it keeps. A Mac that cannot run the local
/// model dictates in the cloud, and the cloud needs a ChatGPT session so the
/// first recording has somewhere to go.
fn finish_onboarding(
    settings: &mut AppSettings,
    local_runs: bool,
    signed_in: bool,
) -> Result<(), String> {
    if !local_runs {
        settings.transcription_engine = Engine::Cloud;
    }
    if settings.transcription_engine == Engine::Cloud && !signed_in {
        return Err("No ChatGPT session was found. Sign in with ChatGPT and retry.".to_string());
    }
    settings.onboarding_completed = true;
    // Onboarding already offered local transcription; the card is for
    // users from before it.
    settings.local_stt_card_dismissed = true;
    Ok(())
}

#[tauri::command]
#[specta::specta]
/// Dictation on this Mac needs no account. The cloud needs a Codex session,
/// so the first recording has somewhere to go.
pub async fn complete_onboarding(app: AppHandle) -> Result<(), String> {
    let signed_in =
        tauri::async_runtime::spawn_blocking(|| crate::codex_transcribe::auth_status().signed_in)
            .await
            .map_err(|error| format!("Failed to inspect transcription configuration: {error}"))?;
    let local_runs = runnable_models(&app).is_ok();
    crate::settings::update_settings_checked(&app, |settings| {
        finish_onboarding(settings, local_runs, signed_in)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    /// Finishes a new install's onboarding after it chose `engine`, and
    /// returns the settings it keeps.
    fn finish(engine: Engine, local_runs: bool, signed_in: bool) -> Result<AppSettings, String> {
        let mut settings = get_default_settings();
        settings.transcription_engine = engine;
        finish_onboarding(&mut settings, local_runs, signed_in)?;
        assert!(settings.onboarding_completed);
        Ok(settings)
    }

    fn engine(finished: Result<AppSettings, String>) -> Engine {
        finished.unwrap().transcription_engine
    }

    #[test]
    fn onboarding_needs_an_account_only_for_the_cloud() {
        assert_eq!(engine(finish(Engine::Local, true, false)), Engine::Local);
        assert_eq!(engine(finish(Engine::Local, true, true)), Engine::Local);
        assert_eq!(engine(finish(Engine::Cloud, true, true)), Engine::Cloud);
        assert!(finish(Engine::Cloud, true, false).is_err());
    }

    #[test]
    fn a_mac_that_cannot_run_the_local_model_finishes_in_the_cloud() {
        assert_eq!(engine(finish(Engine::Local, false, true)), Engine::Cloud);
        assert!(finish(Engine::Local, false, false).is_err());
    }

    /// The card announces local transcription to users from before it. A new
    /// install was offered it in onboarding, so choosing the cloud there must
    /// not bring the offer back on the first screen.
    #[test]
    fn a_new_install_never_sees_the_local_transcription_card() {
        for (engine, local_runs) in [
            (Engine::Cloud, true),
            (Engine::Local, true),
            (Engine::Local, false),
        ] {
            let settings = finish(engine, local_runs, true).unwrap();
            assert!(
                settings.local_stt_card_dismissed,
                "{engine:?}, local runs: {local_runs}"
            );
        }
    }
}
