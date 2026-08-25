use crate::accent::{self, NativeIconState};
use crate::managers::history::{HistoryEntry, HistoryManager};
use crate::settings;
use crate::tray_i18n::get_tray_translations;
use log::{debug, error, info, warn};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIcon;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayIconState {
    Idle,
    Recording,
    Transcribing,
}

/// Tauri managed state holding the last icon state set via `change_tray_icon`.
pub struct CurrentTrayIconState(pub Mutex<TrayIconState>);

impl CurrentTrayIconState {
    pub fn new() -> Self {
        Self(Mutex::new(TrayIconState::Idle))
    }

    pub fn get(&self) -> TrayIconState {
        *self.0.lock().unwrap()
    }

    fn set(&self, state: TrayIconState) {
        *self.0.lock().unwrap() = state;
    }
}

pub fn change_tray_icon(app: &AppHandle, icon: TrayIconState) {
    let tray = app.state::<TrayIcon>();
    let accent_color = settings::get_settings(app).accent_color;

    // Store current state
    app.state::<CurrentTrayIconState>().set(icon);

    let icon_started = std::time::Instant::now();
    let native_state = match icon {
        TrayIconState::Idle => NativeIconState::Idle,
        TrayIconState::Recording => NativeIconState::Recording,
        TrayIconState::Transcribing => NativeIconState::Transcribing,
    };
    match accent::tray_icon(accent_color, native_state) {
        Ok(image) => {
            if let Err(error) =
                tray.set_icon_with_as_template(Some(image), accent::tray_icon_is_template())
            {
                error!("Failed to update tray icon: {error}");
            }
        }
        Err(error) => error!("Failed to build tray icon: {error}"),
    }
    let icon_elapsed = icon_started.elapsed();

    // Update menu based on state
    let menu_started = std::time::Instant::now();
    update_tray_menu(app, None);
    debug!(
        "tray icon change ({:?}): accent={:?} set_icon={:?} menu={:?}",
        icon,
        accent_color,
        icon_elapsed,
        menu_started.elapsed()
    );
}

/// Re-applies the last known tray state — for when only the *theme* changed
/// and the state itself (idle/recording/transcribing) should be preserved.
pub fn refresh_tray_icon(app: &AppHandle) {
    let icon = app.state::<CurrentTrayIconState>().get();
    change_tray_icon(app, icon);
}

pub fn tray_tooltip() -> String {
    version_label()
}

fn version_label() -> String {
    if cfg!(debug_assertions) {
        format!("Murmur v{} (Dev)", env!("CARGO_PKG_VERSION"))
    } else {
        format!("Murmur v{}", env!("CARGO_PKG_VERSION"))
    }
}

pub fn update_tray_menu(app: &AppHandle, locale: Option<&str>) {
    let state = app.state::<CurrentTrayIconState>().get();
    let settings = settings::get_settings(app);

    let locale = locale.unwrap_or(&settings.app_language);
    let strings = get_tray_translations(Some(locale.to_string()));

    // Platform-specific accelerators
    #[cfg(target_os = "macos")]
    let (settings_accelerator, quit_accelerator) = (Some("Cmd+,"), Some("Cmd+Q"));
    #[cfg(not(target_os = "macos"))]
    let (settings_accelerator, quit_accelerator) = (Some("Ctrl+,"), Some("Ctrl+Q"));

    // Create common menu items
    let version_label = version_label();
    let version_i = MenuItem::with_id(app, "version", &version_label, false, None::<&str>)
        .expect("failed to create version item");
    let settings_i = MenuItem::with_id(
        app,
        "settings",
        &strings.settings,
        true,
        settings_accelerator,
    )
    .expect("failed to create settings item");
    let check_updates_i = MenuItem::with_id(
        app,
        "check_updates",
        &strings.check_updates,
        settings.update_checks_enabled,
        None::<&str>,
    )
    .expect("failed to create check updates item");
    let copy_last_transcript_i = MenuItem::with_id(
        app,
        "copy_last_transcript",
        &strings.copy_last_transcript,
        true,
        None::<&str>,
    )
    .expect("failed to create copy last transcript item");
    let quit_i = MenuItem::with_id(app, "quit", &strings.quit, true, quit_accelerator)
        .expect("failed to create quit item");
    let separator = || PredefinedMenuItem::separator(app).expect("failed to create separator");

    let menu = match state {
        TrayIconState::Recording | TrayIconState::Transcribing => {
            let cancel_i = MenuItem::with_id(app, "cancel", &strings.cancel, true, None::<&str>)
                .expect("failed to create cancel item");
            Menu::with_items(
                app,
                &[
                    &version_i,
                    &separator(),
                    &cancel_i,
                    &separator(),
                    &copy_last_transcript_i,
                    &separator(),
                    &settings_i,
                    &check_updates_i,
                    &separator(),
                    &quit_i,
                ],
            )
            .expect("failed to create menu")
        }
        TrayIconState::Idle => Menu::with_items(
            app,
            &[
                &version_i,
                &separator(),
                &copy_last_transcript_i,
                &separator(),
                &settings_i,
                &check_updates_i,
                &separator(),
                &quit_i,
            ],
        )
        .expect("failed to create menu"),
    };

    let tray = app.state::<TrayIcon>();
    let _ = tray.set_menu(Some(menu));
    let _ = tray.set_tooltip(Some(version_label));
}

fn last_transcript_text(entry: &HistoryEntry) -> &str {
    &entry.transcription_text
}

pub fn set_tray_visibility(app: &AppHandle, visible: bool) {
    let tray = app.state::<TrayIcon>();
    if let Err(e) = tray.set_visible(visible) {
        error!("Failed to set tray visibility: {}", e);
    } else {
        info!("Tray visibility set to: {}", visible);
    }
}

pub fn copy_last_transcript(app: &AppHandle) {
    let history_manager = app.state::<Arc<HistoryManager>>();
    let entry = match history_manager.get_latest_completed_entry() {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            warn!("No completed transcription history entries available for tray copy.");
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch last completed transcription entry: {}",
                err
            );
            return;
        }
    };

    let text = last_transcript_text(&entry);
    if text.trim().is_empty() {
        warn!("Last completed transcription is empty; skipping tray copy.");
        return;
    }

    if let Err(err) = app.clipboard().write_text(text) {
        error!("Failed to copy last transcript to clipboard: {}", err);
        return;
    }

    info!("Copied last transcript to clipboard via tray.");
}

#[cfg(test)]
mod tests {
    use super::last_transcript_text;
    use crate::managers::history::HistoryEntry;

    fn build_entry(transcription: &str) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            file_name: "murmur-1.wav".to_string(),
            timestamp: 0,
            saved: false,
            title: "Recording".to_string(),
            transcription_text: transcription.to_string(),
        }
    }

    #[test]
    fn uses_transcription_text() {
        let entry = build_entry("raw");
        assert_eq!(last_transcript_text(&entry), "raw");
    }
}
