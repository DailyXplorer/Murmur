//! Meeting recording: the microphone ("Me") and system audio (the remote
//! participants) on two tracks, started and stopped by the user, kept in a
//! library of their own. Everything here stays behind `meetings_enabled`.

pub mod cap;
pub mod capture;
pub mod coordinator;
pub mod paths;
pub mod store;
pub mod wav_recovery;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use log::{error, info};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::settings::get_settings;
use crate::user_alert::{self, Alert};
use capture::system::SystemAudioSupport;
use capture::DeviceCapture;
use coordinator::{Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier};
use store::{MeetingsPage, MeetingsStore};

pub const BINDING_ID: &str = "meeting_toggle";
pub const DEFAULT_BINDING: &str = "option+command+m";
pub const TRAY_ITEM_ID: &str = "meeting_toggle";

/// Sent whenever a meeting starts or stops.
#[derive(Serialize, Debug, Clone, Type, Event)]
pub struct MeetingStateChanged(pub MeetingRuntime);

#[derive(Serialize, Debug, Clone, Copy, Type)]
pub struct MeetingCapabilities {
    pub system_audio: SystemAudioSupport,
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// Shows the recording state everywhere it appears. The coordinator calls
/// it with its lock held, so nothing here waits on the main thread.
struct AppNotifier {
    app: AppHandle,
    runtime: Arc<Mutex<MeetingRuntime>>,
}

impl Notifier for AppNotifier {
    fn runtime_changed(&self, runtime: &MeetingRuntime) {
        *self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = runtime.clone();
        match runtime {
            MeetingRuntime::Idle => crate::overlay::hide_meeting_overlay(&self.app),
            MeetingRuntime::Recording {
                started_at_ms,
                stops_at_ms,
                ..
            } => crate::overlay::show_meeting_overlay(&self.app, *started_at_ms, *stops_at_ms),
        }
        let app = self.app.clone();
        let _ = self
            .app
            .run_on_main_thread(move || crate::utils::update_tray_menu(&app, None));
        if let Err(error) = MeetingStateChanged(runtime.clone()).emit(&self.app) {
            error!("Failed to emit the meeting state: {error}");
        }
    }

    fn alert(&self, alert: MeetingAlert) {
        user_alert::show(&self.app, Alert::Meeting(alert));
    }
}

type AppCoordinator = Coordinator<DeviceCapture, AppNotifier, SystemClock>;

pub struct MeetingsManager {
    app: AppHandle,
    coordinator: Mutex<AppCoordinator>,
    runtime: Arc<Mutex<MeetingRuntime>>,
    store: Arc<MeetingsStore>,
}

impl MeetingsManager {
    /// Opens the library and marks meetings a crash cut off as interrupted.
    pub fn new(app: &AppHandle) -> anyhow::Result<Self> {
        let app_data = app.path().app_data_dir()?;
        let root: PathBuf = app_data.join("meetings");
        std::fs::create_dir_all(&root)?;
        let (store, startup) =
            MeetingsStore::open(app_data.join("meetings.db"), chrono::Utc::now());
        info!("Meetings database: {startup:?}");
        match store.recover_interrupted(&root) {
            Ok(0) => {}
            Ok(count) => info!("Recovered {count} interrupted meeting(s)"),
            Err(error) => error!("Failed to recover interrupted meetings: {error}"),
        }
        let store = Arc::new(store);
        let runtime = Arc::new(Mutex::new(MeetingRuntime::Idle));
        let notifier = AppNotifier {
            app: app.clone(),
            runtime: Arc::clone(&runtime),
        };
        Ok(Self {
            app: app.clone(),
            coordinator: Mutex::new(Coordinator::new(
                DeviceCapture,
                notifier,
                SystemClock,
                Arc::clone(&store),
                root,
            )),
            runtime,
            store,
        })
    }

    fn coordinator(&self) -> std::sync::MutexGuard<'_, AppCoordinator> {
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn runtime(&self) -> MeetingRuntime {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn is_recording(&self) -> bool {
        self.runtime() != MeetingRuntime::Idle
    }

    pub fn start(&self) -> Result<(), String> {
        self.start_with(&mut self.coordinator())
    }

    fn start_with(&self, coordinator: &mut AppCoordinator) -> Result<(), String> {
        let settings = get_settings(&self.app);
        if !settings.meetings_enabled {
            return Err("Meetings are turned off".to_string());
        }
        let id = coordinator
            .start(settings.meeting_capture_mode)
            .map_err(|error| error.to_string())?;
        info!("Meeting {id} started");
        self.spawn_ticker(id);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        Self::stop_with(&mut self.coordinator())
    }

    fn stop_with(coordinator: &mut AppCoordinator) -> Result<(), String> {
        let id = coordinator.stop().map_err(|error| error.to_string())?;
        info!("Meeting {id} recorded");
        Ok(())
    }

    pub fn discard(&self) -> Result<(), String> {
        self.coordinator()
            .discard()
            .map_err(|error| error.to_string())
    }

    /// Decides under the lock, so two quick presses never start two meetings.
    pub fn toggle(&self) -> Result<(), String> {
        let mut coordinator = self.coordinator();
        if coordinator.runtime() == MeetingRuntime::Idle {
            self.start_with(&mut coordinator)
        } else {
            Self::stop_with(&mut coordinator)
        }
    }

    pub fn list(&self, cursor: Option<i64>, limit: usize) -> Result<MeetingsPage, String> {
        self.store
            .list(cursor, limit)
            .map_err(|error| error.to_string())
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.coordinator()
            .delete(id)
            .map_err(|error| error.to_string())
    }

    /// Keeps a meeting in progress when the app quits: its tracks are closed
    /// and it waits for processing.
    pub fn shutdown(&self) {
        if self.is_recording() {
            if let Err(error) = self.stop() {
                error!("Failed to finish the meeting on quit: {error}");
            }
        }
    }

    /// Drives the cap and write-failure checks of meeting `id` until it ends.
    fn spawn_ticker(&self, id: i64) {
        let app = self.app.clone();
        let spawned = std::thread::Builder::new()
            .name("meeting-ticker".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
                    return;
                };
                if !manager.coordinator().tick(id) {
                    return;
                }
            });
        if let Err(error) = spawned {
            error!("Failed to start the meeting timer: {error}");
        }
    }
}

/// Opens the meetings library once. At startup that happens only when
/// meetings are on or a library exists, so a user who never turned them on
/// gets no new files.
pub fn init(app: &AppHandle, at_startup: bool) {
    if app.try_state::<Arc<MeetingsManager>>().is_some() {
        return;
    }
    if at_startup && !get_settings(app).meetings_enabled {
        let exists = app
            .path()
            .app_data_dir()
            .is_ok_and(|dir| dir.join("meetings.db").exists());
        if !exists {
            return;
        }
    }
    match MeetingsManager::new(app) {
        Ok(manager) => {
            app.manage(Arc::new(manager));
        }
        Err(error) => error!("Failed to initialize meetings: {error}"),
    }
}

pub fn is_recording(app: &AppHandle) -> bool {
    app.try_state::<Arc<MeetingsManager>>()
        .is_some_and(|manager| manager.is_recording())
}

/// Starts or stops a meeting from the shortcut or the tray. Opening the
/// devices takes a moment, so it runs off the calling (often main) thread.
pub fn request_toggle(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
            return;
        };
        let starting = !manager.is_recording();
        if let Err(error) = manager.toggle() {
            error!("Meeting toggle failed: {error}");
            if starting {
                user_alert::show(&app, Alert::Meeting(MeetingAlert::StartFailed));
            }
        }
    });
}

pub struct MeetingToggleAction;

impl crate::actions::ShortcutAction for MeetingToggleAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        request_toggle(app);
    }

    fn stop(
        &self,
        _app: &AppHandle,
        _binding_id: &str,
        _shortcut_str: &str,
        _operation: crate::ProcessingOperation,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binding_is_a_valid_shortcut() {
        crate::shortcut::tauri_impl::parse_shortcut(DEFAULT_BINDING).unwrap();
    }
}
