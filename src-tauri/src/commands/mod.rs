pub mod audio;
pub mod chatgpt;
pub mod cleanup;
pub mod history;
pub mod local_models;
pub mod meetings;
pub mod transcription;

use crate::settings::{get_settings, write_settings, AppSettings, LogLevel};
use crate::utils::cancel_current_operation_before_coordinator;
use crate::TranscriptionCoordinator;
use std::process::Command;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

const TCCUTIL_PATH: &str = "/usr/bin/tccutil";
const OPEN_PATH: &str = "/usr/bin/open";
const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const MICROPHONE_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

fn accessibility_reset_arguments(bundle_id: &str) -> Result<[String; 3], String> {
    if bundle_id.trim().is_empty() {
        return Err("Cannot repair Accessibility without an application bundle ID".to_string());
    }

    Ok([
        "reset".to_string(),
        "Accessibility".to_string(),
        bundle_id.to_string(),
    ])
}

fn reset_accessibility_decision(bundle_id: &str) -> Result<(), String> {
    let arguments = accessibility_reset_arguments(bundle_id)?;
    let output = Command::new(TCCUTIL_PATH)
        .args(arguments)
        .output()
        .map_err(|error| format!("Failed to start Accessibility repair: {error}"))?;

    if output.status.success() {
        log::info!("Reset the stale Accessibility decision for {bundle_id}");
        return Ok(());
    }

    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if detail.is_empty() {
        Err(format!(
            "Accessibility repair exited with status {}",
            output.status
        ))
    } else {
        Err(format!("Accessibility repair failed: {detail}"))
    }
}

fn open_privacy_pane(url: &str, name: &str) -> Result<(), String> {
    let status = Command::new(OPEN_PATH)
        .arg(url)
        .status()
        .map_err(|error| format!("Failed to open {name} settings: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Opening {name} settings exited with status {status}"
        ))
    }
}

#[tauri::command]
#[specta::specta]
pub fn cancel_operation(app: AppHandle) {
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.send_cancel();
    } else {
        cancel_current_operation_before_coordinator(&app);
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_app_dir_path(app: AppHandle) -> Result<String, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    Ok(app_data_dir.to_string_lossy().to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    Ok(get_settings(&app))
}

#[tauri::command]
#[specta::specta]
pub fn get_default_settings() -> Result<AppSettings, String> {
    Ok(crate::settings::get_default_settings())
}

#[tauri::command]
#[specta::specta]
pub fn get_log_dir_path(app: AppHandle) -> Result<String, String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    Ok(log_dir.to_string_lossy().to_string())
}

#[specta::specta]
#[tauri::command]
pub fn set_log_level(app: AppHandle, level: LogLevel) -> Result<(), String> {
    let tauri_log_level: tauri_plugin_log::LogLevel = level.into();
    let log_level: log::Level = tauri_log_level.into();
    crate::FILE_LOG_LEVEL.store(
        log_level.to_level_filter() as u8,
        std::sync::atomic::Ordering::Relaxed,
    );

    let mut settings = get_settings(&app);
    settings.log_level = level;
    write_settings(&app, settings);

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_recordings_folder(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    let recordings_dir = app_data_dir.join("recordings");

    let path = recordings_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open recordings folder: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_log_dir(app: AppHandle) -> Result<(), String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("Failed to get log directory: {}", e))?;

    let path = log_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open log directory: {}", e))?;

    Ok(())
}

#[specta::specta]
#[tauri::command]
pub fn open_app_data_dir(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;

    let path = app_data_dir.to_string_lossy().as_ref().to_string();
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| format!("Failed to open app data directory: {}", e))?;

    Ok(())
}

/// Replace Murmur's stale Accessibility entry after an ad-hoc signed update.
///
/// macOS ties Accessibility decisions to the app's code requirement. Ad hoc
/// signed builds (local builds, and releases made before the stable signing
/// certificate) change that requirement on every update even though the
/// bundle ID and installation path stay the same.
/// Reset only this running bundle's Accessibility decision, register the current
/// process with the system prompt, and take the user straight to the correct
/// privacy pane. Microphone and every other privacy decision are left untouched.
#[specta::specta]
#[tauri::command]
pub async fn repair_accessibility_permission(app: AppHandle) -> Result<(), String> {
    if tauri_plugin_macos_permissions::check_accessibility_permission().await {
        return Ok(());
    }

    let bundle_id = app.config().identifier.clone();
    let reset_bundle_id = bundle_id.clone();
    let reset_result = tauri::async_runtime::spawn_blocking(move || {
        reset_accessibility_decision(&reset_bundle_id)
    })
    .await
    .map_err(|error| format!("Accessibility repair task failed: {error}"))?;

    tauri_plugin_macos_permissions::request_accessibility_permission().await;

    let open_result = tauri::async_runtime::spawn_blocking(|| {
        open_privacy_pane(ACCESSIBILITY_SETTINGS_URL, "Accessibility")
    })
    .await
    .map_err(|error| format!("Accessibility settings task failed: {error}"))?;

    reset_result?;
    open_result?;

    Ok(())
}

/// Open the Microphone privacy pane, where the user can turn access back on.
/// After a denial macOS no longer shows the access prompt.
#[specta::specta]
#[tauri::command]
pub async fn open_microphone_settings() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        open_privacy_pane(MICROPHONE_SETTINGS_URL, "Microphone")
    })
    .await
    .map_err(|error| format!("Microphone settings task failed: {error}"))?
}

/// The raw `AVAuthorizationStatus` for audio capture: 0 not determined,
/// 1 restricted, 2 denied, 3 authorized.
#[specta::specta]
#[tauri::command]
pub fn get_microphone_authorization_status() -> Result<i32, String> {
    use objc2::{msg_send, runtime::AnyClass};
    use objc2_foundation::NSString;

    let device = AnyClass::get(c"AVCaptureDevice")
        .ok_or_else(|| "AVFoundation is not loaded".to_string())?;
    let audio = NSString::from_str("soun");
    let status: isize = unsafe { msg_send![device, authorizationStatusForMediaType: &*audio] };
    i32::try_from(status).map_err(|_| format!("Unknown microphone authorization status {status}"))
}

/// Try to initialize Enigo (keyboard/mouse simulation).
/// On macOS, this will return an error if accessibility permissions are not granted.
#[specta::specta]
#[tauri::command]
pub fn initialize_enigo(app: AppHandle) -> Result<(), String> {
    use crate::input::EnigoState;

    if app.try_state::<EnigoState>().is_some() {
        log::debug!("Enigo already initialized");
        return Ok(());
    }

    match EnigoState::new() {
        Ok(enigo_state) => {
            app.manage(enigo_state);
            log::info!("Enigo initialized successfully after permission grant");
            Ok(())
        }
        Err(e) => {
            log::warn!(
                "Failed to initialize Enigo: {} (accessibility permissions may not be granted)",
                e
            );
            Err(format!("Failed to initialize input system: {}", e))
        }
    }
}

/// Marker state to track if shortcuts have been initialized.
pub struct ShortcutsInitialized;

/// Initialize keyboard shortcuts.
/// On macOS, this should be called after accessibility permissions are granted.
/// This is idempotent - calling it multiple times is safe.
#[specta::specta]
#[tauri::command]
pub fn initialize_shortcuts(app: AppHandle) -> Result<(), String> {
    if app.try_state::<ShortcutsInitialized>().is_some() {
        log::debug!("Shortcuts already initialized");
        return Ok(());
    }

    crate::shortcut::init_shortcuts(&app);
    app.manage(ShortcutsInitialized);

    log::info!("Shortcuts initialized successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        accessibility_reset_arguments, get_microphone_authorization_status,
        ACCESSIBILITY_SETTINGS_URL, MICROPHONE_SETTINGS_URL, OPEN_PATH, TCCUTIL_PATH,
    };

    #[test]
    fn microphone_authorization_status_is_read_without_the_webview() {
        let status = get_microphone_authorization_status().unwrap();
        assert!((0..=3).contains(&status), "status {status}");
    }

    #[test]
    fn accessibility_repair_targets_only_the_running_bundle() {
        assert_eq!(TCCUTIL_PATH, "/usr/bin/tccutil");
        assert_eq!(
            accessibility_reset_arguments("com.dailyxplorer.murmur").unwrap(),
            [
                "reset".to_string(),
                "Accessibility".to_string(),
                "com.dailyxplorer.murmur".to_string(),
            ]
        );
    }

    #[test]
    fn accessibility_repair_rejects_a_missing_bundle_id() {
        assert!(accessibility_reset_arguments("  ").is_err());
    }

    #[test]
    fn accessibility_repair_uses_absolute_system_tools_and_privacy_pane() {
        assert_eq!(OPEN_PATH, "/usr/bin/open");
        assert!(ACCESSIBILITY_SETTINGS_URL.ends_with("Privacy_Accessibility"));
        assert!(MICROPHONE_SETTINGS_URL.ends_with("Privacy_Microphone"));
    }
}
