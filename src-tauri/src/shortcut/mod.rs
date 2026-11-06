//! Global shortcut registration and settings commands.

mod handler;
mod key_labels;
#[cfg(target_os = "windows")]
mod modifier_chord;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
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
    let settings = settings::get_settings(app);
    for id in MEETING_BINDINGS {
        if meeting_shortcut_yields(&settings, id) {
            record_registration_error(app, id, ShortcutRegistrationError::UsedByAnotherAction);
        }
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

#[tauri::command]
#[specta::specta]
/// What the active keyboard layout writes on the keys shortcuts name, keyed
/// by the name, so a shortcut shows the key to press. Empty when the system
/// cannot tell, and the names are shown as they are.
pub async fn get_key_labels(app: AppHandle) -> HashMap<String, String> {
    read_keyboard_layout(&app, key_labels::read).await
}

#[tauri::command]
#[specta::specta]
/// The name to record for a key the user presses, keyed by the name of the
/// physical key, so the shortcut fires on that key, and the keys no shortcut
/// can fire on. Only keys whose name changes are listed: none on macOS, which
/// registers the physical key.
pub async fn get_recorded_key_names(app: AppHandle) -> key_labels::RecordedKeys {
    read_keyboard_layout(&app, key_labels::read_names).await
}

/// Runs `read` on the main thread, where the layout APIs answer. Empty when
/// the layout cannot be read.
async fn read_keyboard_layout<T: Default + Send + 'static>(
    app: &AppHandle,
    read: fn() -> Result<T, String>,
) -> T {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Err(error) = app.run_on_main_thread(move || {
        let _ = sender.send(read());
    }) {
        warn!("Cannot read the keyboard layout: {error}");
        return T::default();
    }
    match receiver.await {
        Ok(Ok(keys)) => keys,
        Ok(Err(error)) => {
            warn!("Cannot read the keyboard layout: {error}");
            T::default()
        }
        Err(_) => T::default(),
    }
}

/// The shortcuts that exist only while meetings are on.
pub const MEETING_BINDINGS: [&str; 2] = [
    crate::meetings::BINDING_ID,
    crate::meetings::live::PASTE_BINDING_ID,
];

/// Whether `id` is in use: the meeting shortcuts exist only while meetings
/// are on. A binding that is not in use never blocks another one's keys.
pub fn binding_active(settings: &settings::AppSettings, id: &str) -> bool {
    !MEETING_BINDINGS.contains(&id) || settings.meetings_enabled
}

/// Whether the meeting shortcut `id` gives up its keys because another action
/// already uses them, for example an older dictation shortcut. The other
/// action always keeps them, whatever order the shortcuts register in.
pub fn meeting_shortcut_yields(settings: &settings::AppSettings, id: &str) -> bool {
    MEETING_BINDINGS.contains(&id)
        && binding_active(settings, id)
        && settings.bindings.get(id).is_some_and(|meeting| {
            tauri_impl::conflicting_binding(settings, id, &meeting.current_binding).is_some()
        })
}

/// Whether `id` holds a global hotkey of its own. Cancel is registered only
/// while recording; a meeting shortcut only while meetings are on and its
/// keys are free.
pub fn binding_registers(settings: &settings::AppSettings, id: &str) -> bool {
    id != "cancel"
        && binding_active(settings, id)
        && !meeting_shortcut_yields(settings, id)
        && settings
            .bindings
            .get(id)
            .is_none_or(|binding| !binding.current_binding.trim().is_empty())
}

fn may_be_unbound(id: &str) -> bool {
    settings::get_default_settings()
        .bindings
        .get(id)
        .is_some_and(|binding| binding.default_binding.is_empty())
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

/// Held across every command that registers hotkeys from the settings, so a
/// resume cannot read the bindings a rebind is about to replace and register
/// the old keys again. Only async commands take it, never the main thread.
static REGISTRATIONS: Mutex<()> = Mutex::new(());

fn hold_registrations() -> MutexGuard<'static, ()> {
    REGISTRATIONS.lock().unwrap_or_else(PoisonError::into_inner)
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
    let _registrations = hold_registrations();
    if !(binding.trim().is_empty() && may_be_unbound(&id)) {
        tauri_impl::validate_binding(&binding)?;
    }

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
    } else if held {
        let _ = unregister_shortcut(&app, previous.clone());
    }

    clear_registration_error(&app, &id);
    give_back_meeting_keys(&app, &current, &next, &id);
    settings::update_settings(&app, |settings| {
        settings.bindings.insert(id, updated.clone());
    });
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
    meeting: &str,
) -> bool {
    id != meeting
        && meeting_shortcut_yields(current, meeting)
        && !meeting_shortcut_yields(next, meeting)
}

fn give_back_meeting_keys(
    app: &AppHandle,
    current: &settings::AppSettings,
    next: &settings::AppSettings,
    id: &str,
) {
    for meeting in MEETING_BINDINGS {
        if !frees_meeting_keys(current, next, id, meeting) {
            continue;
        }
        if let Some(binding) = next.bindings.get(meeting).cloned() {
            match register_shortcut(app, binding) {
                Ok(()) => clear_registration_error(app, meeting),
                Err(_) => {
                    record_registration_error(app, meeting, ShortcutRegistrationError::Unavailable)
                }
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

/// True from the settings page's suspend to its resume: a field captures keys
/// and every hotkey is off. A page destroyed in between never resumes them.
static BINDINGS_SUSPENDED: AtomicBool = AtomicBool::new(false);

#[tauri::command(async)]
#[specta::specta]
pub fn suspend_all_bindings(app: AppHandle) -> Result<(), String> {
    let _registrations = hold_registrations();
    suspend_all_shortcuts(&app);
    BINDINGS_SUSPENDED.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command(async)]
#[specta::specta]
pub fn resume_all_bindings(app: AppHandle) -> Result<(), String> {
    let _registrations = hold_registrations();
    BINDINGS_SUSPENDED.store(false, Ordering::SeqCst);
    resume_all_shortcuts(&app);
    Ok(())
}

/// Puts the hotkeys back if the page that suspended them is gone. Takes the
/// registration lock, so it runs off the main thread.
pub fn resume_bindings_if_suspended(app: &AppHandle) {
    if BINDINGS_SUSPENDED.load(Ordering::SeqCst) {
        let _ = resume_all_bindings(app.clone());
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.audio_feedback = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_volume_setting(app: AppHandle, volume: f32) -> Result<(), String> {
    settings::update_settings(&app, |value| {
        value.audio_feedback_volume = volume.clamp(0.0, 1.0)
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_sound_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let parsed = match theme.as_str() {
        "marimba" => SoundTheme::Marimba,
        "pop" => SoundTheme::Pop,
        "custom" => SoundTheme::Custom,
        _ => return Err(format!("Invalid sound theme: {theme}")),
    };
    settings::update_settings(&app, |value| value.sound_theme = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let parsed = match theme.as_str() {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ => return Err(format!("Invalid theme: {theme}")),
    };
    settings::update_settings(&app, |value| value.theme = parsed);
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

    settings::update_settings(&app, |value| value.accent_color = parsed);

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
    settings::update_settings(&app, |value| value.selected_language = language);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_position_setting(app: AppHandle, position: String) -> Result<(), String> {
    let parsed = match position.as_str() {
        "top" => OverlayPosition::Top,
        "bottom" => OverlayPosition::Bottom,
        _ => return Err(format!("Invalid overlay position: {position}")),
    };
    settings::update_settings(&app, |value| value.overlay_position = parsed);
    crate::utils::update_overlay_position(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_style_setting(app: AppHandle, style: String) -> Result<(), String> {
    let parsed = match style.as_str() {
        "none" => OverlayStyle::None,
        "minimal" => OverlayStyle::Minimal,
        _ => return Err(format!("Invalid overlay style: {style}")),
    };
    settings::update_settings(&app, |value| value.overlay_style = parsed);
    crate::overlay::update_overlay_enabled_cache(parsed != OverlayStyle::None);
    crate::utils::update_overlay_position(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_debug_mode_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.debug_mode = enabled);
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
    settings::update_settings(&app, |value| value.start_hidden = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_autostart_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.autostart_enabled = enabled);
    crate::autostart::apply_autostart(&app, enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_update_checks_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.update_checks_enabled = enabled);
    // The tray's "Check for Updates…" item follows the setting.
    tray::update_tray_menu(&app, None);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_whats_new_on_update_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    settings::update_settings(&app, |value| value.show_whats_new_on_update = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_whats_new_last_seen_version_setting(
    app: AppHandle,
    version: String,
) -> Result<(), String> {
    settings::update_settings(&app, |value| {
        value.whats_new_last_seen_version = version.trim().to_string()
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn update_custom_words(app: AppHandle, words: Vec<String>) -> Result<(), String> {
    settings::validate_custom_words(&words)?;
    settings::update_settings(&app, |value| value.custom_words = words);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_word_correction_threshold_setting(
    app: AppHandle,
    threshold: f64,
) -> Result<(), String> {
    settings::update_settings(&app, |value| {
        value.word_correction_threshold = threshold.clamp(0.0, 1.0)
    });
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
    settings::update_settings(&app, |value| value.extra_recording_buffer_ms = ms);
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
    settings::update_settings(&app, |value| value.paste_delay_ms = ms);
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
    settings::update_settings(&app, |value| value.paste_delay_after_ms = ms);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_reliable_paste_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    if !enabled {
        return Err("Reliable paste is required to protect clipboard contents".to_string());
    }
    settings::update_settings(&app, |value| value.reliable_paste = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_method_setting(app: AppHandle, method: String) -> Result<(), String> {
    let parsed = match method.as_str() {
        "ctrl_v" => PasteMethod::CtrlV,
        "direct" => PasteMethod::Direct,
        "none" => PasteMethod::None,
        _ => return Err(format!("Invalid paste method: {method}")),
    };
    settings::update_settings(&app, |value| value.paste_method = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_handling_setting(app: AppHandle, handling: String) -> Result<(), String> {
    let parsed = match handling.as_str() {
        "dont_modify" => ClipboardHandling::DontModify,
        "copy_to_clipboard" => ClipboardHandling::CopyToClipboard,
        _ => return Err(format!("Invalid clipboard handling: {handling}")),
    };
    settings::update_settings(&app, |value| value.clipboard_handling = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.auto_submit = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_key_setting(app: AppHandle, key: String) -> Result<(), String> {
    let parsed = match key.as_str() {
        "enter" => AutoSubmitKey::Enter,
        "ctrl_enter" => AutoSubmitKey::CtrlEnter,
        "cmd_enter" => AutoSubmitKey::CmdEnter,
        _ => return Err(format!("Invalid auto-submit key: {key}")),
    };
    settings::update_settings(&app, |value| value.auto_submit_key = parsed);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_experimental_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.experimental_enabled = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_mute_while_recording_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.mute_while_recording = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_leading_space_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.leading_space = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_append_trailing_space_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.append_trailing_space = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_lazy_stream_close_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.lazy_stream_close = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_app_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    settings::update_settings(&app, |value| value.app_language = language.clone());
    tray::update_tray_menu(&app, Some(&language));
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_tray_icon_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    settings::update_settings(&app, |value| value.show_tray_icon = enabled);
    tray::set_tray_visibility(&app, enabled);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        binding_registers, frees_meeting_keys, meeting_shortcut_yields,
        tauri_impl::conflicting_binding, ShortcutRegistrationError, MEETING_BINDINGS,
    };
    use crate::meetings::live::PASTE_BINDING_ID;
    use crate::meetings::{BINDING_ID, DEFAULT_BINDING};

    const MEETING_KEYS: &str = "ctrl+shift+f8";

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

    fn meeting_keys_taken_by(meeting: &str, id: &str) -> crate::settings::AppSettings {
        let mut settings = crate::settings::get_default_settings();
        settings.meetings_enabled = false;
        bound(&mut settings, meeting, MEETING_KEYS);
        bound(&mut settings, id, MEETING_KEYS);
        settings.meetings_enabled = true;
        settings
    }

    #[test]
    fn an_action_already_on_the_meeting_keys_keeps_them_when_meetings_turn_on() {
        for meeting in MEETING_BINDINGS {
            for id in ["transcribe", "paste_last", "cancel"] {
                let mut settings = meeting_keys_taken_by(meeting, id);

                assert!(
                    meeting_shortcut_yields(&settings, meeting),
                    "{meeting} {id}"
                );
                assert!(!binding_registers(&settings, meeting), "{meeting} {id}");
                assert_eq!(binding_registers(&settings, id), id != "cancel");
                // Spelled differently, the keys are the same.
                bound(&mut settings, meeting, "control+shift+f8");
                assert!(
                    meeting_shortcut_yields(&settings, meeting),
                    "{meeting} {id}"
                );

                bound(&mut settings, id, "ctrl+shift+f9");
                assert!(
                    !meeting_shortcut_yields(&settings, meeting),
                    "{meeting} {id}"
                );
                assert!(binding_registers(&settings, meeting), "{meeting} {id}");
            }
        }
    }

    #[test]
    fn moving_any_other_action_off_the_meeting_keys_gives_them_back() {
        for meeting in MEETING_BINDINGS {
            for id in ["transcribe", "paste_last", "cancel"] {
                let current = meeting_keys_taken_by(meeting, id);
                let mut next = current.clone();
                bound(&mut next, id, "ctrl+shift+f9");
                assert!(
                    frees_meeting_keys(&current, &next, id, meeting),
                    "{meeting} {id}"
                );
            }
        }
    }

    #[test]
    fn new_keys_for_a_yielded_meeting_shortcut_are_registered_only_once() {
        for meeting in MEETING_BINDINGS {
            let current = meeting_keys_taken_by(meeting, "transcribe");
            let mut next = current.clone();
            bound(&mut next, meeting, "ctrl+shift+f9");

            assert!(binding_registers(&next, meeting), "{meeting}");
            assert!(
                !frees_meeting_keys(&current, &next, meeting, meeting),
                "{meeting}"
            );
        }
    }

    #[test]
    fn registration_errors_serialize_as_the_frontend_reads_them() {
        assert_eq!(
            serde_json::to_value(ShortcutRegistrationError::UsedByAnotherAction).unwrap(),
            "used_by_another_action"
        );
    }

    #[test]
    fn the_meeting_shortcuts_hold_their_keys_only_while_meetings_are_on() {
        let mut settings = crate::settings::get_default_settings();
        bound(&mut settings, PASTE_BINDING_ID, MEETING_KEYS);
        settings.meetings_enabled = false;
        assert!(binding_registers(&settings, "transcribe"));
        assert!(!binding_registers(&settings, "cancel"));
        for meeting in MEETING_BINDINGS {
            assert!(!binding_registers(&settings, meeting), "{meeting}");
        }
        settings.meetings_enabled = true;
        for meeting in MEETING_BINDINGS {
            assert!(binding_registers(&settings, meeting), "{meeting}");
        }
        assert!(!binding_registers(&settings, "cancel"));
    }

    #[test]
    fn the_paste_shortcut_ships_unbound_so_it_holds_no_keys() {
        let mut settings = crate::settings::get_default_settings();
        settings.meetings_enabled = true;
        assert!(!binding_registers(&settings, PASTE_BINDING_ID));
    }
}
