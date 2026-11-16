use crate::insights::categories::AppCategory;
use crate::settings::{self, AppSettings, CategoryStyle, DictationStyle};
use std::collections::BTreeMap;
use tauri::AppHandle;

fn update(app: &AppHandle, change: impl FnOnce(&mut AppSettings)) -> Result<(), String> {
    settings::update_settings(app, change);
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
pub fn change_category_styles_setting(
    app: AppHandle,
    styles: BTreeMap<AppCategory, CategoryStyle>,
) -> Result<(), String> {
    update(&app, |value| value.category_styles = styles)
}

/// Each language is "auto" or a code, checked the way a one-off
/// re-transcription language is, so a bad one never reaches an engine.
#[tauri::command]
#[specta::specta]
pub fn change_category_languages_setting(
    app: AppHandle,
    languages: BTreeMap<AppCategory, String>,
) -> Result<(), String> {
    for language in languages.values() {
        super::history::checked_language(Some(language.clone()))?;
    }
    update(&app, |value| value.category_languages = languages)
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

#[tauri::command]
#[specta::specta]
pub fn change_cleanup_short_dictations_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    update(&app, |value| value.cleanup_short_dictations = enabled)
}
