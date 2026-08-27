//! Global shortcut registration and settings commands.

mod handler;
pub mod tauri_impl;

use crate::accent;
use crate::settings::{
    self, AccentColor, AutoSubmitKey, ClipboardHandling, OverlayPosition, OverlayStyle,
    PasteMethod, ShortcutBinding, SoundTheme, Theme, TranscriptionProvider, TypingTool,
};
use crate::tray;
use log::{debug, warn};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};

pub fn init_shortcuts(app: &AppHandle) {
    tauri_impl::init_shortcuts(app);
}

pub fn register_cancel_shortcut(app: &AppHandle) {
    tauri_impl::register_cancel_shortcut(app);
}

pub fn unregister_cancel_shortcut(app: &AppHandle) {
    tauri_impl::unregister_cancel_shortcut(app);
}

pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    tauri_impl::register_shortcut(app, binding)
}

pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    tauri_impl::unregister_shortcut(app, binding)
}

#[derive(Serialize, Type)]
pub struct BindingResponse {
    success: bool,
    binding: Option<ShortcutBinding>,
    error: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn change_binding(
    app: AppHandle,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    tauri_impl::validate_shortcut(&binding)?;
    let mut current = settings::get_settings(&app);
    let previous = current
        .bindings
        .get(&id)
        .cloned()
        .or_else(|| settings::get_default_settings().bindings.get(&id).cloned())
        .ok_or_else(|| format!("Binding '{id}' does not exist"))?;

    if id == "cancel" {
        let mut updated = previous;
        updated.current_binding = binding;
        current.bindings.insert(id, updated.clone());
        settings::write_settings(&app, current);
        return Ok(BindingResponse {
            success: true,
            binding: Some(updated),
            error: None,
        });
    }

    if let Err(error) = unregister_shortcut(&app, previous.clone()) {
        debug!("Previous shortcut was not registered: {error}");
    }

    let mut updated = previous.clone();
    updated.current_binding = binding;
    if let Err(error) = register_shortcut(&app, updated.clone()) {
        let _ = register_shortcut(&app, previous);
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error),
        });
    }

    current.bindings.insert(id, updated.clone());
    settings::write_settings(&app, current);
    Ok(BindingResponse {
        success: true,
        binding: Some(updated),
        error: None,
    })
}

#[tauri::command]
#[specta::specta]
pub fn reset_binding(app: AppHandle, id: String) -> Result<BindingResponse, String> {
    let default = settings::get_default_settings()
        .bindings
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("Binding '{id}' does not exist"))?;
    change_binding(app, id, default.default_binding)
}

pub fn suspend_all_shortcuts(app: &AppHandle) {
    for (id, binding) in settings::get_bindings(app) {
        if id != "cancel" {
            let _ = unregister_shortcut(app, binding);
        }
    }
}

pub fn resume_all_shortcuts(app: &AppHandle) {
    for (id, binding) in settings::get_bindings(app) {
        if id != "cancel" {
            let _ = register_shortcut(app, binding);
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn suspend_all_bindings(app: AppHandle) -> Result<(), String> {
    suspend_all_shortcuts(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn resume_all_bindings(app: AppHandle) -> Result<(), String> {
    resume_all_shortcuts(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_ptt_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.push_to_talk = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.audio_feedback = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_volume_setting(app: AppHandle, volume: f32) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.audio_feedback_volume = volume.clamp(0.0, 1.0);
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_sound_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.sound_theme = match theme.as_str() {
        "marimba" => SoundTheme::Marimba,
        "pop" => SoundTheme::Pop,
        "custom" => SoundTheme::Custom,
        _ => return Err(format!("Invalid sound theme: {theme}")),
    };
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    let parsed = match theme.as_str() {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ => return Err(format!("Invalid theme: {theme}")),
    };
    value.theme = parsed;
    settings::write_settings(&app, value);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    apply_window_theme(&app, parsed);
    let _ = app.emit("theme-changed", parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_accent_color_setting(app: AppHandle, accent_color: String) -> Result<(), String> {
    let parsed = match accent_color.as_str() {
        "pink" => AccentColor::Pink,
        "blue" => AccentColor::Blue,
        "green" => AccentColor::Green,
        "yellow" => AccentColor::Yellow,
        "orange" => AccentColor::Orange,
        "red" => AccentColor::Red,
        _ => return Err(format!("Invalid accent color: {accent_color}")),
    };

    let mut value = settings::get_settings(&app);
    value.accent_color = parsed;
    settings::write_settings(&app, value);

    if let Err(error) = accent::apply_native_accent(&app, parsed) {
        warn!("Failed to update native accent icon: {error}");
    }
    tray::refresh_tray_icon(&app);
    let _ = app.emit("accent-color-changed", parsed);
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn apply_window_theme(app: &AppHandle, theme: Theme) {
    let theme = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_theme(theme) {
            warn!("Failed to apply window theme: {error}");
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_selected_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.selected_language = language;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_transcription_provider_setting(
    app: AppHandle,
    provider: TranscriptionProvider,
) -> Result<(), String> {
    if provider == TranscriptionProvider::Gemini {
        #[cfg(not(target_os = "macos"))]
        return Err("Gemini transcription is currently available on macOS only.".to_string());

        #[cfg(target_os = "macos")]
        {
            let status = crate::gemini_transcribe::status();
            if !status.installed {
                return Err(
                    "Antigravity is not installed. Install it before selecting Gemini transcription."
                        .to_string(),
                );
            }
            if !status.signed_in {
                return Err(
                    "No Antigravity session was found. Open Antigravity, sign in, and retry."
                        .to_string(),
                );
            }
        }
    }

    let mut value = settings::get_settings(&app);
    value.transcription_provider = provider;
    settings::write_settings(&app, value);
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "transcription_provider"}),
    );
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_position_setting(app: AppHandle, position: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.overlay_position = match position.as_str() {
        "top" => OverlayPosition::Top,
        "bottom" => OverlayPosition::Bottom,
        _ => return Err(format!("Invalid overlay position: {position}")),
    };
    settings::write_settings(&app, value);
    crate::utils::update_overlay_position(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_style_setting(app: AppHandle, style: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    let parsed = match style.as_str() {
        "none" => OverlayStyle::None,
        "minimal" => OverlayStyle::Minimal,
        _ => return Err(format!("Invalid overlay style: {style}")),
    };
    value.overlay_style = parsed;
    settings::write_settings(&app, value);
    crate::overlay::update_overlay_enabled_cache(parsed != OverlayStyle::None);
    crate::utils::update_overlay_position(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_debug_mode_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.debug_mode = enabled;
    settings::write_settings(&app, value);
    crate::WEBVIEW_LOG_STREAMING.store(enabled, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "debug_mode", "value": enabled}),
    );
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_start_hidden_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.start_hidden = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_autostart_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.autostart_enabled = enabled;
    settings::write_settings(&app, value);
    crate::autostart::apply_autostart(&app, enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_update_checks_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.update_checks_enabled = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_whats_new_on_update_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.show_whats_new_on_update = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_whats_new_last_seen_version_setting(
    app: AppHandle,
    version: String,
) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.whats_new_last_seen_version = version.trim().to_string();
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn update_custom_words(app: AppHandle, words: Vec<String>) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.custom_words = words;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_word_correction_threshold_setting(
    app: AppHandle,
    threshold: f64,
) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.word_correction_threshold = threshold.clamp(0.0, 1.0);
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_extra_recording_buffer_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.extra_recording_buffer_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.paste_delay_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_after_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.paste_delay_after_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_reliable_paste_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.reliable_paste = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_method_setting(app: AppHandle, method: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.paste_method = match method.as_str() {
        "ctrl_v" => PasteMethod::CtrlV,
        "direct" => PasteMethod::Direct,
        "none" => PasteMethod::None,
        "shift_insert" => PasteMethod::ShiftInsert,
        "ctrl_shift_v" => PasteMethod::CtrlShiftV,
        "external_script" => PasteMethod::ExternalScript,
        _ => return Err(format!("Invalid paste method: {method}")),
    };
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_available_typing_tools() -> Vec<String> {
    vec!["auto".to_string()]
}

#[tauri::command]
#[specta::specta]
pub fn change_typing_tool_setting(app: AppHandle, tool: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.typing_tool = match tool.as_str() {
        "auto" => TypingTool::Auto,
        "wtype" => TypingTool::Wtype,
        "kwtype" => TypingTool::Kwtype,
        "dotool" => TypingTool::Dotool,
        "ydotool" => TypingTool::Ydotool,
        "xdotool" => TypingTool::Xdotool,
        _ => return Err(format!("Invalid typing tool: {tool}")),
    };
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_external_script_path_setting(
    app: AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.external_script_path = path;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_handling_setting(app: AppHandle, handling: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.clipboard_handling = match handling.as_str() {
        "dont_modify" => ClipboardHandling::DontModify,
        "copy_to_clipboard" => ClipboardHandling::CopyToClipboard,
        _ => return Err(format!("Invalid clipboard handling: {handling}")),
    };
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.auto_submit = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_key_setting(app: AppHandle, key: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.auto_submit_key = match key.as_str() {
        "enter" => AutoSubmitKey::Enter,
        "ctrl_enter" => AutoSubmitKey::CtrlEnter,
        "cmd_enter" => AutoSubmitKey::CmdEnter,
        _ => return Err(format!("Invalid auto-submit key: {key}")),
    };
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_experimental_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.experimental_enabled = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_mute_while_recording_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.mute_while_recording = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_append_trailing_space_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.append_trailing_space = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_lazy_stream_close_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.lazy_stream_close = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_filler_word_removal_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.filler_word_removal_enabled = enabled;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_app_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.app_language = language.clone();
    settings::write_settings(&app, value);
    tray::update_tray_menu(&app, Some(&language));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_tray_icon_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut value = settings::get_settings(&app);
    value.show_tray_icon = enabled;
    settings::write_settings(&app, value);
    tray::set_tray_visibility(&app, enabled);
    Ok(())
}
