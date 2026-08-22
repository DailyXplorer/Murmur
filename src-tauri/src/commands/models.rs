use crate::managers::model::{ModelInfo, ModelManager};
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_available_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<Vec<ModelInfo>, String> {
    Ok(model_manager.get_available_models())
}

#[tauri::command]
#[specta::specta]
pub async fn get_model_info(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<Option<ModelInfo>, String> {
    Ok(model_manager.get_model_info(&model_id))
}

pub fn switch_active_model(app: &AppHandle, model_id: &str) -> Result<(), String> {
    let model_manager = app.state::<Arc<ModelManager>>();
    let transcription_manager = app.state::<Arc<TranscriptionManager>>();
    let _loading_guard = transcription_manager
        .try_start_loading()
        .ok_or_else(|| "Model load already in progress".to_string())?;
    let _model_info = model_manager
        .get_model_info(model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;
    let settings = get_settings(app);
    let old_model = settings.selected_model.clone();
    let old_onboarding_completed = settings.onboarding_completed;
    let mut settings = settings;
    settings.selected_model = model_id.to_string();
    settings.onboarding_completed = true;
    write_settings(app, settings);
    if let Err(e) = transcription_manager.load_model(model_id) {
        let mut settings = get_settings(app);
        settings.selected_model = old_model;
        settings.onboarding_completed = old_onboarding_completed;
        write_settings(app, settings);
        return Err(e.to_string());
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_active_model(
    app_handle: AppHandle,
    _model_manager: State<'_, Arc<ModelManager>>,
    _transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
) -> Result<(), String> {
    switch_active_model(&app_handle, &model_id)
}

#[tauri::command]
#[specta::specta]
pub async fn get_current_model(app_handle: AppHandle) -> Result<String, String> {
    let settings = get_settings(&app_handle);
    Ok(settings.selected_model)
}
