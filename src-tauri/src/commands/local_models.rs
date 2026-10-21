use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::local_models::{LocalModelStatus, LocalModels};

fn models(app: &AppHandle) -> Result<Arc<LocalModels>, String> {
    app.try_state::<Arc<LocalModels>>()
        .map(|models| Arc::clone(&models))
        .ok_or_else(|| "The local models are unavailable".to_string())
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
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_local_model(app: AppHandle) -> Result<(), String> {
    let models = models(&app)?;
    // Removing about 500 MB of files takes a moment.
    tauri::async_runtime::spawn_blocking(move || models.delete())
        .await
        .map_err(|error| error.to_string())?
}
