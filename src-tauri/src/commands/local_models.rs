use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::engine::local::LocalEngine;
use crate::local_models::{LocalModelState, LocalModelStatus, LocalModels};
use crate::settings::update_settings;

pub(crate) fn models(app: &AppHandle) -> Result<Arc<LocalModels>, String> {
    app.try_state::<Arc<LocalModels>>()
        .map(|models| Arc::clone(&models))
        .ok_or_else(|| "The local models are unavailable".to_string())
}

/// The local models, refused when this Mac cannot run them.
pub(crate) fn runnable_models(app: &AppHandle) -> Result<Arc<LocalModels>, String> {
    let models = models(app)?;
    match models.status().state {
        LocalModelState::Unsupported { .. } => {
            Err("This Mac cannot run the local model".to_string())
        }
        _ => Ok(models),
    }
}

/// Stopping or deleting the model also drops a pending switch to it.
fn forget_switch(app: &AppHandle) {
    let was_switching = update_settings(app, |settings| {
        let was_switching = settings.switch_to_local_when_ready;
        settings.switch_to_local_when_ready = false;
        was_switching
    });
    if was_switching {
        announce_switch_dropped(app);
    }
}

/// The one-time card reads the pending switch from the windows' copy of the
/// settings, which only a `settings-changed` event refreshes.
pub(crate) fn announce_switch_dropped(app: &AppHandle) {
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "switch_to_local_when_ready"}),
    );
}

#[tauri::command]
#[specta::specta]
pub fn get_local_model_status(app: AppHandle) -> Result<LocalModelStatus, String> {
    Ok(models(&app)?.status())
}

#[tauri::command]
#[specta::specta]
pub fn download_local_model(app: AppHandle) -> Result<(), String> {
    models(&app)?.download();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn cancel_local_model_download(app: AppHandle) -> Result<(), String> {
    models(&app)?.cancel();
    forget_switch(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(app: AppHandle) -> Result<(), String> {
    let engine = app
        .try_state::<Arc<LocalEngine>>()
        .map(|engine| Arc::clone(&engine));
    delete(models(&app)?, engine.as_deref()).await?;
    forget_switch(&app);
    Ok(())
}

/// Deletes the models once the dictation model is released, which loads
/// again only after the files are gone: Windows may refuse to delete the file
/// of a loaded model.
pub(crate) async fn delete(
    models: Arc<LocalModels>,
    engine: Option<&LocalEngine>,
) -> Result<(), String> {
    // Removing about 500 MB of files takes a moment.
    let delete = async move {
        tauri::async_runtime::spawn_blocking(move || models.delete())
            .await
            .map_err(|error| error.to_string())?
    };
    match engine {
        Some(engine) => engine.release(delete).await,
        None => delete.await,
    }
}
