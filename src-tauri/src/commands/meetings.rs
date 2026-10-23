use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::meetings::coordinator::MeetingRuntime;
use crate::meetings::paths::MeetingsStorage;
use crate::meetings::store::{MeetingDetail, MeetingsPage};
use crate::meetings::{self, MeetingCapabilities, MeetingMarkdownKind, MeetingsManager};
use crate::settings::{
    get_settings, update_settings, AppSettings, MeetingAudioRetention, MeetingCaptureMode,
    ShortcutBinding,
};
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

fn page_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(PAGE_SIZE).clamp(1, PAGE_SIZE)
}

#[tauri::command]
#[specta::specta]
pub async fn get_meetings(
    app: AppHandle,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<MeetingsPage, String> {
    let limit = page_limit(limit);
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
        call_detection: meetings::call_watch::supported(),
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

/// What turning meetings on or off does to the meeting shortcut.
enum MeetingShortcutChange {
    /// Another action already uses these keys and keeps them; the shortcut
    /// field says so until the user picks other keys.
    UsedByAnotherAction,
    Register(ShortcutBinding),
    /// Only a shortcut that held its keys gives them back.
    Release {
        binding: ShortcutBinding,
        unregister: bool,
    },
}

struct MeetingsToggle {
    settings: AppSettings,
    /// `None` when there is no meeting shortcut.
    shortcut: Option<MeetingShortcutChange>,
}

/// `None` when meetings are already `enabled`.
fn toggle_meetings(mut settings: AppSettings, enabled: bool) -> Option<MeetingsToggle> {
    if settings.meetings_enabled == enabled {
        return None;
    }
    let held = crate::shortcut::binding_registers(&settings, meetings::BINDING_ID);
    settings.meetings_enabled = enabled;
    let yields = crate::shortcut::meeting_shortcut_yields(&settings);
    let shortcut = settings
        .bindings
        .get(meetings::BINDING_ID)
        .cloned()
        .map(|binding| {
            if yields {
                MeetingShortcutChange::UsedByAnotherAction
            } else if enabled {
                MeetingShortcutChange::Register(binding)
            } else {
                MeetingShortcutChange::Release {
                    binding,
                    unregister: held,
                }
            }
        });
    Some(MeetingsToggle { settings, shortcut })
}

/// Turning meetings off stops a meeting in progress (it is kept) and frees
/// the shortcut's keys; turning them on claims them.
#[tauri::command]
#[specta::specta]
pub fn change_meetings_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let Some(toggle) = toggle_meetings(get_settings(&app), enabled) else {
        return Ok(());
    };
    if enabled {
        meetings::init(&app, false);
    }
    // Saved before the shutdown: a meeting still opening its devices sees
    // meetings off once it has started, and discards itself. Only this field
    // belongs to the toggle; its earlier planning snapshot is never persisted.
    update_settings(&app, |settings| {
        settings.meetings_enabled = toggle.settings.meetings_enabled;
    });
    if let Some(manager) = app.try_state::<Arc<MeetingsManager>>() {
        if enabled {
            manager.resume_processing();
        } else {
            manager.shutdown();
        }
    }

    match toggle.shortcut {
        Some(MeetingShortcutChange::UsedByAnotherAction) => {
            crate::shortcut::record_registration_error(
                &app,
                meetings::BINDING_ID,
                crate::shortcut::ShortcutRegistrationError::UsedByAnotherAction,
            );
        }
        Some(MeetingShortcutChange::Register(binding)) => {
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
        }
        Some(MeetingShortcutChange::Release {
            binding,
            unregister,
        }) => {
            crate::shortcut::clear_registration_error(&app, meetings::BINDING_ID);
            if unregister {
                if let Err(error) = crate::shortcut::unregister_shortcut(&app, binding) {
                    log::debug!("Meeting shortcut was not registered: {error}");
                }
            }
        }
        None => {}
    }
    meetings::call_watch::sync(&app);
    crate::utils::update_tray_menu(&app, None);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_call_prompt_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update_settings(&app, |settings| settings.meeting_call_prompt = enabled);
    meetings::call_watch::sync(&app);
    Ok(())
}

/// The call prompt's buttons: "Record" when `accept`, else "Not now".
#[tauri::command]
#[specta::specta]
pub fn answer_call_prompt(app: AppHandle, accept: bool) {
    meetings::call_watch::answer(&app, accept);
}

fn with_capture_mode(settings: &mut AppSettings, mode: MeetingCaptureMode) {
    settings.meeting_capture_mode = mode;
}

#[tauri::command]
#[specta::specta]
pub fn change_meeting_capture_mode_setting(
    app: AppHandle,
    mode: MeetingCaptureMode,
) -> Result<(), String> {
    update_settings(&app, |settings| with_capture_mode(settings, mode));
    Ok(())
}

/// Takes effect at the next daily pass, or at the next launch.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_audio_retention_setting(
    app: AppHandle,
    retention: MeetingAudioRetention,
) -> Result<(), String> {
    update_settings(&app, |settings| {
        settings.meeting_audio_retention = retention
    });
    Ok(())
}

/// Measured on disk, off the main thread: uncompressed tracks count too.
#[tauri::command]
#[specta::specta]
pub async fn get_meetings_storage(app: AppHandle) -> Result<MeetingsStorage, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || meetings::paths::storage(&app_data))
        .await
        .map_err(|error| format!("Measuring the meetings failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{page_limit, toggle_meetings, with_capture_mode, MeetingShortcutChange};
    use crate::meetings::{BINDING_ID, DEFAULT_BINDING};
    use crate::settings::{get_default_settings, AppSettings, MeetingCaptureMode};

    fn meetings(enabled: bool) -> AppSettings {
        let mut settings = get_default_settings();
        settings.meetings_enabled = enabled;
        settings
    }

    fn on_meeting_keys(mut settings: AppSettings, id: &str) -> AppSettings {
        settings.bindings.get_mut(id).unwrap().current_binding = DEFAULT_BINDING.to_string();
        settings
    }

    fn shortcut(settings: AppSettings, enabled: bool) -> MeetingShortcutChange {
        toggle_meetings(settings, enabled)
            .expect("a change")
            .shortcut
            .expect("a meeting shortcut")
    }

    #[test]
    fn a_page_holds_one_to_fifty_meetings() {
        assert_eq!(page_limit(None), 50);
        assert_eq!(page_limit(Some(0)), 1);
        assert_eq!(page_limit(Some(20)), 20);
        assert_eq!(page_limit(Some(500)), 50);
    }

    #[test]
    fn an_unchanged_toggle_does_nothing() {
        for enabled in [true, false] {
            assert!(
                toggle_meetings(meetings(enabled), enabled).is_none(),
                "{enabled}"
            );
            assert!(
                toggle_meetings(meetings(enabled), !enabled).is_some(),
                "{enabled}"
            );
        }
    }

    #[test]
    fn a_toggle_writes_the_new_state() {
        for enabled in [true, false] {
            let toggle = toggle_meetings(meetings(!enabled), enabled).expect("a change");
            assert_eq!(toggle.settings.meetings_enabled, enabled);
        }
    }

    #[test]
    fn turning_meetings_on_with_free_keys_registers_the_shortcut() {
        match shortcut(meetings(false), true) {
            MeetingShortcutChange::Register(binding) => {
                assert_eq!(binding.id, BINDING_ID);
                assert_eq!(binding.current_binding, DEFAULT_BINDING);
            }
            _ => panic!("expected Register"),
        }
    }

    #[test]
    fn turning_meetings_on_leaves_keys_another_action_holds() {
        for id in ["transcribe", "paste_last", "cancel"] {
            let settings = on_meeting_keys(meetings(false), id);
            assert!(
                matches!(
                    shortcut(settings, true),
                    MeetingShortcutChange::UsedByAnotherAction
                ),
                "{id}"
            );
        }
    }

    #[test]
    fn turning_meetings_off_unregisters_a_shortcut_that_held_its_keys() {
        match shortcut(meetings(true), false) {
            MeetingShortcutChange::Release {
                binding,
                unregister,
            } => {
                assert_eq!(binding.id, BINDING_ID);
                assert!(unregister);
            }
            _ => panic!("expected Release"),
        }
    }

    /// The other action keeps its keys, so nothing is unregistered, but the
    /// "used by another action" error is cleared with the shortcut.
    #[test]
    fn turning_meetings_off_releases_a_yielded_shortcut_without_unregistering() {
        for id in ["transcribe", "paste_last", "cancel"] {
            let settings = on_meeting_keys(meetings(true), id);
            assert!(
                matches!(
                    shortcut(settings, false),
                    MeetingShortcutChange::Release {
                        unregister: false,
                        ..
                    }
                ),
                "{id}"
            );
        }
    }

    #[test]
    fn without_a_meeting_shortcut_the_toggle_changes_no_keys() {
        for enabled in [true, false] {
            let mut settings = meetings(!enabled);
            settings.bindings.remove(BINDING_ID);
            let toggle = toggle_meetings(settings, enabled).expect("a change");
            assert!(toggle.shortcut.is_none(), "{enabled}");
        }
    }

    #[test]
    fn capture_mode_changes_only_that_setting() {
        let before = get_default_settings();
        assert_ne!(before.meeting_capture_mode, MeetingCaptureMode::SystemOnly);
        let mut after = before.clone();
        with_capture_mode(&mut after, MeetingCaptureMode::SystemOnly);

        let serde_json::Value::Object(before) = serde_json::to_value(before).unwrap() else {
            panic!("settings serialize as an object");
        };
        let serde_json::Value::Object(after) = serde_json::to_value(after).unwrap() else {
            panic!("settings serialize as an object");
        };
        let changed: Vec<&String> = after
            .keys()
            .filter(|key| before.get(*key) != after.get(*key))
            .collect();
        assert_eq!(changed, ["meeting_capture_mode"]);
        assert_eq!(after["meeting_capture_mode"], "system_only");
        assert_eq!(before.len(), after.len());
    }
}
