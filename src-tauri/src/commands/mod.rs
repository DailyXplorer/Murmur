pub mod audio;
pub mod cleanup;
pub mod history;
pub mod meetings;
pub mod transcription;

use crate::settings::{get_settings, write_settings, AppSettings, LogLevel};
use crate::utils::cancel_current_operation_before_coordinator;
use crate::TranscriptionCoordinator;
#[cfg(target_os = "macos")]
use std::process::Command;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "macos")]
const TCCUTIL_PATH: &str = "/usr/bin/tccutil";
#[cfg(target_os = "macos")]
const OPEN_PATH: &str = "/usr/bin/open";
#[cfg(target_os = "macos")]
const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
#[cfg(target_os = "macos")]
const MICROPHONE_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
/// Settings > Privacy & security > Microphone, from Microsoft's list of
/// `ms-settings:` URIs.
#[cfg(target_os = "windows")]
const MICROPHONE_SETTINGS_URL: &str = "ms-settings:privacy-microphone";

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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

// Windows has no Accessibility permission: any desktop app may send keys to
// apps running at its own privilege level. The doc comment below is the
// macOS one: commands carry it into the bindings both platforms share.
/// Replace Murmur's stale Accessibility entry after an ad-hoc signed update.
///
/// macOS ties Accessibility decisions to the app's code requirement. Ad hoc
/// signed builds (local builds, and releases made before the stable signing
/// certificate) change that requirement on every update even though the
/// bundle ID and installation path stay the same.
/// Reset only this running bundle's Accessibility decision, register the current
/// process with the system prompt, and take the user straight to the correct
/// privacy pane. Microphone and every other privacy decision are left untouched.
#[cfg(target_os = "windows")]
#[specta::specta]
#[tauri::command]
pub async fn repair_accessibility_permission(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

/// Open the Microphone privacy pane, where the user can turn access back on.
/// After a denial macOS no longer shows the access prompt.
#[cfg(target_os = "macos")]
#[specta::specta]
#[tauri::command]
pub async fn open_microphone_settings() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        open_privacy_pane(MICROPHONE_SETTINGS_URL, "Microphone")
    })
    .await
    .map_err(|error| format!("Microphone settings task failed: {error}"))?
}

// Opens the Microphone privacy page of Windows Settings, where desktop apps
// get their microphone access back: Windows never prompts for it. The doc
// comment is the macOS one, shared through the bindings.
/// Open the Microphone privacy pane, where the user can turn access back on.
/// After a denial macOS no longer shows the access prompt.
#[cfg(target_os = "windows")]
#[specta::specta]
#[tauri::command]
pub async fn open_microphone_settings() -> Result<(), String> {
    tauri_plugin_opener::open_url(MICROPHONE_SETTINGS_URL, None::<&str>)
        .map_err(|error| format!("Failed to open Microphone settings: {error}"))
}

/// The raw `AVAuthorizationStatus` for audio capture: 0 not determined,
/// 1 restricted, 2 denied, 3 authorized.
#[cfg(target_os = "macos")]
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

/// One microphone switch of Windows Settings, as its consent store records
/// it: `Value` is "Allow" or "Deny", and a switch never touched has none.
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Consent {
    Allow,
    Deny,
    Unset,
}

#[cfg(target_os = "windows")]
fn read_consent(root: winreg::HKEY, path: &str) -> Consent {
    let value = winreg::RegKey::predef(root)
        .open_subkey(path)
        .and_then(|key| key.get_value::<String, _>("Value"));
    match value.map(|value| value.to_ascii_lowercase()).as_deref() {
        Ok("allow") => Consent::Allow,
        Ok("deny") => Consent::Deny,
        _ => Consent::Unset,
    }
}

/// The macOS status code for Windows' microphone switches: the device-wide
/// one, the per-user one for apps, and "Let desktop apps access your
/// microphone", the one that governs Murmur. A per-user "Deny" only blocks
/// desktop apps when their own switch was never set: privacy tools often
/// turn the per-user one off while desktop apps keep their access. Windows
/// has no prompt, so nothing is ever "not determined", and an untouched
/// switch is on, its default.
#[cfg(target_os = "windows")]
fn microphone_status_from_consent(device: Consent, user: Consent, desktop_apps: Consent) -> i32 {
    const DENIED: i32 = 2;
    const AUTHORIZED: i32 = 3;
    let denied = device == Consent::Deny
        || match desktop_apps {
            Consent::Deny => true,
            Consent::Allow => false,
            Consent::Unset => user == Consent::Deny,
        };
    if denied {
        DENIED
    } else {
        AUTHORIZED
    }
}

// The Windows consent in the same codes as macOS, only ever 2 or 3. The doc
// comment is the macOS one, shared through the bindings.
/// The raw `AVAuthorizationStatus` for audio capture: 0 not determined,
/// 1 restricted, 2 denied, 3 authorized.
#[cfg(target_os = "windows")]
#[specta::specta]
#[tauri::command]
pub fn get_microphone_authorization_status() -> Result<i32, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    const MICROPHONE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let desktop_apps = format!(r"{MICROPHONE}\NonPackaged");
    Ok(microphone_status_from_consent(
        read_consent(HKEY_LOCAL_MACHINE, MICROPHONE),
        read_consent(HKEY_CURRENT_USER, MICROPHONE),
        read_consent(HKEY_CURRENT_USER, &desktop_apps),
    ))
}

// Windows sends keys through SendInput and needs no input permission. The
// doc comment is the macOS one, shared through the bindings.
/// Try to initialize Enigo (keyboard/mouse simulation).
/// On macOS, this will return an error if accessibility permissions are not granted.
#[cfg(target_os = "windows")]
#[specta::specta]
#[tauri::command]
pub fn initialize_enigo(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

/// Try to initialize Enigo (keyboard/mouse simulation).
/// On macOS, this will return an error if accessibility permissions are not granted.
#[cfg(target_os = "macos")]
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
    use super::get_microphone_authorization_status;

    #[test]
    fn microphone_authorization_status_is_read_without_the_webview() {
        let status = get_microphone_authorization_status().unwrap();
        assert!((0..=3).contains(&status), "status {status}");
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::{microphone_status_from_consent, Consent};

    #[test]
    fn any_switch_that_governs_desktop_apps_can_deny_the_microphone() {
        use Consent::{Allow, Deny, Unset};
        let status = microphone_status_from_consent;
        assert_eq!(status(Unset, Unset, Unset), 3);
        assert_eq!(status(Allow, Allow, Allow), 3);
        assert_eq!(status(Deny, Allow, Allow), 2);
        assert_eq!(status(Allow, Allow, Deny), 2);
        // The per-user switch denies only through an untouched desktop one.
        assert_eq!(status(Allow, Deny, Unset), 2);
        assert_eq!(status(Allow, Deny, Allow), 3);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::{
        accessibility_reset_arguments, ACCESSIBILITY_SETTINGS_URL, MICROPHONE_SETTINGS_URL,
        OPEN_PATH, TCCUTIL_PATH,
    };

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
