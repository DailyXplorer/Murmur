use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::meetings::coordinator::MeetingRuntime;
use crate::meetings::store::{MeetingDetail, MeetingsPage};
use crate::meetings::{self, MeetingCapabilities, MeetingMarkdownKind, MeetingsManager};
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
pub async fn discard_meeting(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.discard(id)).await
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
pub async fn get_meeting(app: AppHandle, id: i64) -> Result<MeetingDetail, String> {
    with_manager(app, move |manager| manager.detail(id)).await
}

/// An empty title gives the meeting back its default name.
#[tauri::command]
#[specta::specta]
pub async fn rename_meeting(app: AppHandle, id: i64, title: String) -> Result<(), String> {
    with_manager(app, move |manager| manager.rename(id, &title)).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_speaker(
    app: AppHandle,
    id: i64,
    key: String,
    name: String,
) -> Result<(), String> {
    with_manager(app, move |manager| manager.rename_speaker(id, &key, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn retry_meeting(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| {
        manager.retry(id);
        Ok(())
    })
    .await
}

/// Puts the summary or the transcript on the clipboard as Markdown, exactly
/// as Export saves it. The app writes it, not the webview: WebKit before
/// Safari 26.4 refuses a webview clipboard write that waited on a command.
#[tauri::command]
#[specta::specta]
pub async fn copy_meeting_markdown(
    app: AppHandle,
    id: i64,
    kind: MeetingMarkdownKind,
) -> Result<(), String> {
    let (markdown, _) =
        with_manager(app.clone(), move |manager| manager.markdown(id, kind)).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(clipboard_app.clipboard().write_text(markdown));
    })
    .map_err(|error| format!("Cannot reach the clipboard: {error}"))?;
    receiver
        .await
        .map_err(|_| "The clipboard write did not run".to_string())?
        .map_err(|error| format!("Cannot copy the meeting notes: {error}"))
}

/// Asks where to save the summary or the transcript and writes it there.
/// Returns the path, or `None` when the user cancels.
#[tauri::command]
#[specta::specta]
pub async fn export_meeting_markdown(
    app: AppHandle,
    id: i64,
    kind: MeetingMarkdownKind,
) -> Result<Option<String>, String> {
    let (markdown, file_name) =
        with_manager(app.clone(), move |manager| manager.markdown(id, kind)).await?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(meetings::save_panel::ask_save_path(&file_name));
    })
    .map_err(|error| format!("Cannot show the save dialog: {error}"))?;
    let Some(path) = receiver
        .await
        .map_err(|_| "The save dialog closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, markdown)
            .map(|()| Some(path.display().to_string()))
            .map_err(|error| format!("Cannot save the meeting notes: {error}"))
    })
    .await
    .map_err(|error| format!("Export task failed: {error}"))?
}

/// Asks Codex again for a summary that failed.
#[tauri::command]
#[specta::specta]
pub async fn retry_meeting_summary(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.retry_summary(id)).await
}

/// Deletes a finished meeting's audio; its transcript and summary stay.
#[tauri::command]
#[specta::specta]
pub async fn delete_meeting_audio(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.delete_audio(id)).await
}

/// Transcribes a finished meeting again from its audio.
#[tauri::command]
#[specta::specta]
pub async fn retranscribe_meeting(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.retranscribe(id)).await
}

/// Tells the speakers of a finished meeting apart again, for instance once
/// the models are downloaded.
#[tauri::command]
#[specta::specta]
pub async fn reanalyze_meeting(app: AppHandle, id: i64) -> Result<(), String> {
    with_manager(app, move |manager| manager.reanalyze(id)).await
}

#[tauri::command]
#[specta::specta]
pub fn get_meeting_capabilities() -> MeetingCapabilities {
    MeetingCapabilities {
        system_audio: meetings::capture::system::support(),
        analysis: meetings::helper::support(),
    }
}

/// Whether the warning about a silent system track should point to the
/// privacy settings.
#[tauri::command]
#[specta::specta]
pub fn get_system_audio_permission() -> meetings::capture::system::SystemAudioPermission {
    meetings::capture::system::permission()
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
    }
    // Only a shortcut that held its keys gives them back.
    let held = crate::shortcut::binding_registers(&settings, meetings::BINDING_ID);
    settings.meetings_enabled = enabled;
    let binding = settings.bindings.get(meetings::BINDING_ID).cloned();
    let yields = crate::shortcut::meeting_shortcut_yields(&settings);
    // Saved before the shutdown: a meeting still opening its devices sees
    // meetings off once it has started, and discards itself.
    write_settings(&app, settings);
    if let Some(manager) = app.try_state::<Arc<MeetingsManager>>() {
        if enabled {
            manager.resume_processing();
        } else {
            manager.shutdown();
        }
    }

    if let Some(binding) = binding {
        if yields {
            // Another action already uses these keys and keeps them; the
            // shortcut field says so until the user picks other keys.
            crate::shortcut::record_registration_error(
                &app,
                meetings::BINDING_ID,
                crate::shortcut::ShortcutRegistrationError::UsedByAnotherAction,
            );
        } else if enabled {
            if let Err(error) = crate::shortcut::register_shortcut(&app, binding.clone()) {
                log::warn!("Meeting shortcut is unavailable: {error}");
                crate::shortcut::record_registration_error(
                    &app,
                    meetings::BINDING_ID,
                    crate::shortcut::ShortcutRegistrationError::Unavailable,
                );
                user_alert::show(
                    &app,
                    Alert::ShortcutUnavailable {
                        shortcut: binding.current_binding,
                    },
                );
            }
        } else {
            crate::shortcut::clear_registration_error(&app, meetings::BINDING_ID);
            if held {
                if let Err(error) = crate::shortcut::unregister_shortcut(&app, binding) {
                    log::debug!("Meeting shortcut was not registered: {error}");
                }
            }
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
