use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::local_models::{LocalModelState, LocalModelStatus, LocalModels};
use crate::settings::{get_settings, write_settings};

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
    let mut settings = get_settings(app);
    if settings.switch_to_local_when_ready {
        settings.switch_to_local_when_ready = false;
        write_settings(app, settings);
    }
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
    let models = models(&app)?;
    // Removing about 500 MB of files takes a moment.
    tauri::async_runtime::spawn_blocking(move || models.delete())
        .await
        .map_err(|error| error.to_string())??;
    forget_switch(&app);
    Ok(())
}
