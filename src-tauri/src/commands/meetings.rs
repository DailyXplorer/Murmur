use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::meetings::coordinator::MeetingRuntime;
use crate::meetings::store::MeetingsPage;
use crate::meetings::{self, MeetingCapabilities, MeetingsManager};
use crate::settings::{get_settings, write_settings, MeetingCaptureMode};
use crate::user_alert::{self, Alert};

/// "Screen & System Audio Recording", where "System Audio Recording Only"
/// lists the apps allowed to tap system audio.
const SYSTEM_AUDIO_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
const PAGE_SIZE: usize = 50;

/// Runs `action` off the main thread: opening and closing audio devices
/// takes a moment.
async fn with_manager<T: Send + 'static>(
    app: AppHandle,
    action: impl FnOnce(&MeetingsManager) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app
            .try_state::<Arc<MeetingsManager>>()
            .ok_or_else(|| "Meetings are unavailable".to_string())?;
        action(&manager)
    })
    .await
    .map_err(|error| format!("Meeting task failed: {error}"))?
}

#[tauri::command]
#[specta::specta]
pub fn get_meeting_runtime(app: AppHandle) -> MeetingRuntime {
    app.try_state::<Arc<MeetingsManager>>()
        .map(|manager| manager.runtime())
        .unwrap_or(MeetingRuntime::Idle)
}

#[tauri::command]
#[specta::specta]
pub async fn start_meeting(app: AppHandle) -> Result<(), String> {
    with_manager(app, MeetingsManager::start).await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_meeting(app: AppHandle) -> Result<(), String> {
    with_manager(app, MeetingsManager::stop).await
}

#[tauri::command]
#[specta::specta]
pub async fn discard_meeting(app: AppHandle) -> Result<(), String> {
    with_manager(app, MeetingsManager::discard).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_meetings(
    app: AppHandle,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<MeetingsPage, String> {
    let limit = limit.unwrap_or(PAGE_SIZE).clamp(1, PAGE_SIZE);
    with_manager(app, move |manager| manager.list(cursor, limit)).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_meeting(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.delete(id)).await
}

#[tauri::command]
#[specta::specta]
pub fn get_meeting_capabilities() -> MeetingCapabilities {
    MeetingCapabilities {
        system_audio: meetings::capture::system::support(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn open_system_audio_privacy_settings() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        super::open_privacy_pane(SYSTEM_AUDIO_SETTINGS_URL, "System Audio Recording")
    })
    .await
    .map_err(|error| format!("System audio settings task failed: {error}"))?
}

/// Turning meetings off stops a meeting in progress (it is kept) and frees
/// the shortcut's keys; turning them on claims them.
#[tauri::command]
#[specta::specta]
pub fn change_meetings_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    if settings.meetings_enabled == enabled {
        return Ok(());
    }
    if enabled {
        meetings::init(&app, false);
    } else if let Some(manager) = app.try_state::<Arc<MeetingsManager>>() {
        manager.shutdown();
    }
    settings.meetings_enabled = enabled;
    let binding = settings.bindings.get(meetings::BINDING_ID).cloned();
    write_settings(&app, settings);

    if let Some(binding) = binding {
        if enabled {
            if let Err(error) = crate::shortcut::register_shortcut(&app, binding.clone()) {
                log::warn!("Meeting shortcut is unavailable: {error}");
                user_alert::show(
                    &app,
                    Alert::ShortcutUnavailable {
                        shortcut: binding.current_binding,
                    },
                );
            }
        } else if let Err(error) = crate::shortcut::unregister_shortcut(&app, binding) {
            log::debug!("Meeting shortcut was not registered: {error}");
        }
    }
    crate::utils::update_tray_menu(&app, None);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_capture_mode_setting(
    app: AppHandle,
    mode: MeetingCaptureMode,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.meeting_capture_mode = mode;
    write_settings(&app, settings);
    Ok(())
}
