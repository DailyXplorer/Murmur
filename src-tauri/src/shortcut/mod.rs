//! Global shortcut registration and settings commands.

mod handler;
pub mod tauri_impl;

use crate::accent;
use crate::settings::{
    self, AccentColor, AutoSubmitKey, ClipboardHandling, OverlayPosition, OverlayStyle,
    PasteMethod, ShortcutBinding, SoundTheme, Theme,
};
use crate::tray;
use crate::user_alert::{self, Alert};
use log::{debug, warn};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

const REGISTRATION_ERRORS_CHANGED: &str = "shortcut-registration-errors-changed";

/// Why a binding holds no hotkey.
#[derive(Serialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutRegistrationError {
    /// macOS refused the keys, usually because another app holds them.
    Unavailable,
    /// Another Murmur action already uses the keys, and keeps them.
    UsedByAnotherAction,
}

/// Bindings left without a hotkey, by binding id, with the reason. An entry
/// is cleared once the user picks a shortcut that registers.
#[derive(Default)]
pub struct ShortcutRegistrationErrors(Mutex<HashMap<String, ShortcutRegistrationError>>);

impl ShortcutRegistrationErrors {
    fn snapshot(&self) -> HashMap<String, ShortcutRegistrationError> {
        self.0
            .lock()
            .map(|errors| errors.clone())
            .unwrap_or_default()
    }
}

pub fn init_shortcuts(app: &AppHandle) {
    app.manage(ShortcutRegistrationErrors::default());
    let failures = tauri_impl::init_shortcuts(app);
    for (id, binding, _) in failures {
        record_registration_error(app, &id, ShortcutRegistrationError::Unavailable);
        // The meeting shortcut comes with Meetings, on by default: a user who
        // never asked for it hears about its keys only in its settings field.
        if id == crate::meetings::BINDING_ID {
            continue;
        }
        user_alert::show(
            app,
            Alert::ShortcutUnavailable {
                shortcut: binding.current_binding,
            },
        );
    }
    if meeting_shortcut_yields(&settings::get_settings(app)) {
        record_registration_error(
            app,
            crate::meetings::BINDING_ID,
            ShortcutRegistrationError::UsedByAnotherAction,
        );
    }
}

pub fn record_registration_error(app: &AppHandle, id: &str, error: ShortcutRegistrationError) {
    let Some(errors) = app.try_state::<ShortcutRegistrationErrors>() else {
        return;
    };
    if let Ok(mut errors) = errors.0.lock() {
        errors.insert(id.to_string(), error);
    }
    let _ = app.emit(REGISTRATION_ERRORS_CHANGED, ());
}

pub fn clear_registration_error(app: &AppHandle, id: &str) {
    let Some(errors) = app.try_state::<ShortcutRegistrationErrors>() else {
        return;
    };
    let removed = errors
        .0
        .lock()
        .map(|mut errors| errors.remove(id).is_some())
        .unwrap_or(false);
    if removed {
        let _ = app.emit(REGISTRATION_ERRORS_CHANGED, ());
    }
}

#[tauri::command]
#[specta::specta]
/// Shortcuts that hold no hotkey, keyed by binding id.
pub fn get_shortcut_registration_errors(
    app: AppHandle,
) -> HashMap<String, ShortcutRegistrationError> {
    app.try_state::<ShortcutRegistrationErrors>()
        .map(|errors| errors.snapshot())
        .unwrap_or_default()
}

/// Whether `id` is in use: the meeting shortcut exists only while meetings
/// are on. A binding that is not in use never blocks another one's keys.
pub fn binding_active(settings: &settings::AppSettings, id: &str) -> bool {
    id != crate::meetings::BINDING_ID || settings.meetings_enabled
}

/// Whether the meeting shortcut gives up its keys because another action
/// already uses them, for example an older dictation shortcut. The other
/// action always keeps them, whatever order the shortcuts register in.
pub fn meeting_shortcut_yields(settings: &settings::AppSettings) -> bool {
    let id = crate::meetings::BINDING_ID;
    binding_active(settings, id)
        && settings.bindings.get(id).is_some_and(|meeting| {
            tauri_impl::conflicting_binding(settings, id, &meeting.current_binding).is_some()
        })
}

/// Whether `id` holds a global hotkey of its own. Cancel is registered only
/// while recording; the meeting shortcut only while meetings are on and its
/// keys are free.
pub fn binding_registers(settings: &settings::AppSettings, id: &str) -> bool {
    id != "cancel"
        && binding_active(settings, id)
        && !(id == crate::meetings::BINDING_ID && meeting_shortcut_yields(settings))
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

// Async, so off the main thread: a Cancel lifecycle task holds the Cancel
// lock while the shortcut plugin waits on the main thread, and a rebind on
// the main thread waiting for that lock would freeze the app.
#[tauri::command(async)]
#[specta::specta]
pub fn change_binding(
    app: AppHandle,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    tauri_impl::parse_shortcut(&binding)?;

    if let Some(other) =
        tauri_impl::conflicting_binding(&settings::get_settings(&app), &id, &binding)
    {
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(format!(
                "Shortcut '{binding}' is already used by '{}'",
                other.name
            )),
        });
    }

    if id == "cancel" {
        let current = settings::get_settings(&app);
        return match tauri_impl::change_cancel_binding(&app, binding) {
            Ok(updated) => {
                give_back_meeting_keys(&app, &current, &settings::get_settings(&app), &id);
                Ok(BindingResponse {
                    success: true,
                    binding: Some(updated),
                    error: None,
                })
            }
            Err(error) => Ok(BindingResponse {
                success: false,
                binding: None,
                error: Some(error),
            }),
        };
    }

    let current = settings::get_settings(&app);
    let previous = current
        .bindings
        .get(&id)
        .cloned()
        .or_else(|| settings::get_default_settings().bindings.get(&id).cloned())
        .ok_or_else(|| format!("Binding '{id}' does not exist"))?;

    let mut updated = previous.clone();
    updated.current_binding = binding;
    let mut next = current.clone();
    next.bindings.insert(id.clone(), updated.clone());
    // A meeting shortcut that yielded never held its keys: unregistering
    // them would take them from the action that did.
    let held = binding_registers(&current, &id);
    if binding_registers(&next, &id) {
        if held {
            if let Err(error) = unregister_shortcut(&app, previous.clone()) {
                debug!("Previous shortcut was not registered: {error}");
            }
        }
        if let Err(error) = register_shortcut(&app, updated.clone()) {
            if held {
                let _ = register_shortcut(&app, previous);
            }
            return Ok(BindingResponse {
                success: false,
                binding: None,
                error: Some(error),
            });
        }
    }

    clear_registration_error(&app, &id);
    give_back_meeting_keys(&app, &current, &next, &id);
    settings::write_settings(&app, next);
    Ok(BindingResponse {
        success: true,
        binding: Some(updated),
        error: None,
    })
}

/// Whether changing `id` from `current` to `next` moved another action off
/// the keys the meeting shortcut had yielded. A change to the meeting
/// shortcut itself registers its new keys on its own.
fn frees_meeting_keys(
    current: &settings::AppSettings,
    next: &settings::AppSettings,
    id: &str,
) -> bool {
    id != crate::meetings::BINDING_ID
        && meeting_shortcut_yields(current)
        && !meeting_shortcut_yields(next)
}

fn give_back_meeting_keys(
    app: &AppHandle,
    current: &settings::AppSettings,
    next: &settings::AppSettings,
    id: &str,
) {
    if !frees_meeting_keys(current, next, id) {
        return;
    }
    let meeting = crate::meetings::BINDING_ID;
    if let Some(binding) = next.bindings.get(meeting).cloned() {
        match register_shortcut(app, binding) {
            Ok(()) => clear_registration_error(app, meeting),
            Err(_) => {
                record_registration_error(app, meeting, ShortcutRegistrationError::Unavailable)
            }
        }
    }
}

#[tauri::command(async)]
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
    let settings = settings::get_settings(app);
    for (id, binding) in &settings.bindings {
        if binding_registers(&settings, id) {
            let _ = unregister_shortcut(app, binding.clone());
        }
    }
}

pub fn resume_all_shortcuts(app: &AppHandle) {
    let settings = settings::get_settings(app);
    for (id, binding) in &settings.bindings {
        if binding_registers(&settings, id) {
            let _ = register_shortcut(app, binding.clone());
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
    settings::validate_custom_words(&words)?;
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
    if ms > settings::MAX_EXTRA_RECORDING_BUFFER_MS {
        return Err(format!(
            "Recording buffer cannot exceed {} ms",
            settings::MAX_EXTRA_RECORDING_BUFFER_MS
        ));
    }
    let mut value = settings::get_settings(&app);
    value.extra_recording_buffer_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    if ms > settings::MAX_PASTE_DELAY_MS {
        return Err(format!(
            "Paste delay cannot exceed {} ms",
            settings::MAX_PASTE_DELAY_MS
        ));
    }
    let mut value = settings::get_settings(&app);
    value.paste_delay_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_after_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    if ms > settings::MAX_PASTE_DELAY_MS {
        return Err(format!(
            "Paste delay cannot exceed {} ms",
            settings::MAX_PASTE_DELAY_MS
        ));
    }
    let mut value = settings::get_settings(&app);
    value.paste_delay_after_ms = ms;
    settings::write_settings(&app, value);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_reliable_paste_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    if !enabled {
        return Err("Reliable paste is required to protect clipboard contents".to_string());
    }
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
        _ => return Err(format!("Invalid paste method: {method}")),
    };
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

#[cfg(test)]
mod tests {
    use super::{
        binding_registers, frees_meeting_keys, meeting_shortcut_yields,
        tauri_impl::conflicting_binding, ShortcutRegistrationError,
    };
    use crate::meetings::{BINDING_ID, DEFAULT_BINDING};

    fn bound(settings: &mut crate::settings::AppSettings, id: &str, keys: &str) {
        settings.bindings.get_mut(id).unwrap().current_binding = keys.to_string();
    }

    #[test]
    fn with_meetings_off_the_meeting_keys_can_go_to_any_action() {
        let mut settings = crate::settings::get_default_settings();
        settings.meetings_enabled = false;
        for id in ["transcribe", "paste_last", "cancel"] {
            assert!(conflicting_binding(&settings, id, DEFAULT_BINDING).is_none());
        }
        settings.meetings_enabled = true;
        let other = conflicting_binding(&settings, "transcribe", DEFAULT_BINDING);
        assert_eq!(other.map(|binding| binding.id.as_str()), Some(BINDING_ID));
    }

    #[test]
    fn an_action_already_on_the_meeting_keys_keeps_them_when_meetings_turn_on() {
        for id in ["transcribe", "paste_last", "cancel"] {
            let mut settings = crate::settings::get_default_settings();
            settings.meetings_enabled = false;
            bound(&mut settings, id, DEFAULT_BINDING);
            settings.meetings_enabled = true;

            assert!(meeting_shortcut_yields(&settings), "{id}");
            assert!(!binding_registers(&settings, BINDING_ID), "{id}");
            assert_eq!(binding_registers(&settings, id), id != "cancel");
            // Spelled differently, the keys are the same.
            bound(&mut settings, BINDING_ID, "control+alt+cmd+m");
            assert!(meeting_shortcut_yields(&settings), "{id}");

            bound(&mut settings, id, "ctrl+shift+f9");
            assert!(!meeting_shortcut_yields(&settings), "{id}");
            assert!(binding_registers(&settings, BINDING_ID), "{id}");
        }
    }

    #[test]
    fn moving_any_other_action_off_the_meeting_keys_gives_them_back() {
        for id in ["transcribe", "paste_last", "cancel"] {
            let mut current = crate::settings::get_default_settings();
            current.meetings_enabled = false;
            bound(&mut current, id, DEFAULT_BINDING);
            current.meetings_enabled = true;
            let mut next = current.clone();
            bound(&mut next, id, "ctrl+shift+f9");
            assert!(frees_meeting_keys(&current, &next, id), "{id}");
        }
    }

    #[test]
    fn new_keys_for_a_yielded_meeting_shortcut_are_registered_only_once() {
        let mut current = crate::settings::get_default_settings();
        current.meetings_enabled = false;
        bound(&mut current, "transcribe", DEFAULT_BINDING);
        current.meetings_enabled = true;
        let mut next = current.clone();
        bound(&mut next, BINDING_ID, "ctrl+shift+f9");

        assert!(binding_registers(&next, BINDING_ID));
        assert!(!frees_meeting_keys(&current, &next, BINDING_ID));
    }

    #[test]
    fn registration_errors_serialize_as_the_frontend_reads_them() {
        assert_eq!(
            serde_json::to_value(ShortcutRegistrationError::UsedByAnotherAction).unwrap(),
            "used_by_another_action"
        );
    }

    #[test]
    fn the_meeting_shortcut_holds_its_keys_only_while_meetings_are_on() {
        let mut settings = crate::settings::get_default_settings();
        settings.meetings_enabled = false;
        assert!(binding_registers(&settings, "transcribe"));
        assert!(!binding_registers(&settings, "cancel"));
        assert!(!binding_registers(&settings, crate::meetings::BINDING_ID));
        settings.meetings_enabled = true;
        assert!(binding_registers(&settings, crate::meetings::BINDING_ID));
        assert!(!binding_registers(&settings, "cancel"));
    }
}
