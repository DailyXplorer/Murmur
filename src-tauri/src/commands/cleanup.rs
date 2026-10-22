use crate::settings::{self, AppSettings, DictationStyle};
use tauri::AppHandle;

fn update(app: &AppHandle, change: impl FnOnce(&mut AppSettings)) -> Result<(), String> {
    let mut value = settings::get_settings(app);
    change(&mut value);
    settings::write_settings(app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_ai_cleanup_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update(&app, |value| value.ai_cleanup_enabled = enabled)
}

#[tauri::command]
#[specta::specta]
pub fn change_dictation_style_setting(app: AppHandle, style: DictationStyle) -> Result<(), String> {
    update(&app, |value| value.dictation_style = style)
}

#[tauri::command]
#[specta::specta]
pub fn change_window_names_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update(&app, |value| value.window_names_enabled = enabled)
}

#[tauri::command]
#[specta::specta]
pub fn change_whisper_mode_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update(&app, |value| value.whisper_mode = enabled)
}
