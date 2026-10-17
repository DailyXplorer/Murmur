//! Meeting recording: the microphone ("Me") and system audio (the remote
//! participants) on two tracks, started and stopped by the user, kept in a
//! library of their own. Everything here stays behind `meetings_enabled`.

pub mod cap;
pub mod capture;
pub mod chunking;
pub mod coordinator;
pub mod export;
pub mod merge;
pub mod paths;
pub mod processor;
pub mod rename;
pub mod save_panel;
pub mod spread;
pub mod store;
pub mod tokens;
pub mod track_audio;
pub mod transcribe;
pub mod wav_recovery;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::TimeZone;
use log::{error, info};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::settings::get_settings;
use crate::user_alert::{self, Alert};
use crate::{OperationId, ProcessingOperation};
use capture::system::SystemAudioSupport;
use capture::DeviceCapture;
use coordinator::{Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier};
use processor::{MeetingProgress, ProcessEvents, Processor};
use store::{MeetingDetail, MeetingsPage, MeetingsStore, ProcessingError};
use transcribe::{CodexTranscriber, RetryPolicy};

pub const BINDING_ID: &str = "meeting_toggle";
pub const DEFAULT_BINDING: &str = "option+command+m";
pub const TRAY_ITEM_ID: &str = "meeting_toggle";

/// Sent whenever a meeting starts or stops.
#[derive(Serialize, Debug, Clone, Type, Event)]
pub struct MeetingStateChanged(pub MeetingRuntime);

/// A meeting's row or transcript changed: lists and open views reload it.
#[derive(Serialize, Debug, Clone, Copy, Type, Event)]
pub struct MeetingsUpdated {
    pub id: i64,
}

#[derive(Serialize, Debug, Clone, Copy, Type)]
pub struct MeetingCapabilities {
    pub system_audio: SystemAudioSupport,
}

/// Recorded meetings wait here and are processed one at a time, in order.
struct ProcessingQueue {
    sender: tokio::sync::mpsc::UnboundedSender<i64>,
    queued: Mutex<HashSet<i64>>,
    current: Mutex<Option<(i64, ProcessingOperation)>>,
}

impl ProcessingQueue {
    fn enqueue(&self, id: i64) {
        let added = self
            .queued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id);
        if added && self.sender.send(id).is_err() {
            error!("Meeting {id}: the processing queue is closed");
        }
    }

    /// Stops processing meeting `id`, or any meeting when `None`. What was
    /// saved stays, and processing resumes from there next time.
    fn cancel(&self, id: Option<i64>) {
        if let Some((current, operation)) = &*self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            if id.is_none_or(|id| id == *current) {
                operation.cancel();
            }
        }
    }
}

struct AppEvents(AppHandle);

impl ProcessEvents for AppEvents {
    fn progress(&self, progress: MeetingProgress) {
        if let Err(error) = progress.emit(&self.0) {
            error!("Failed to emit meeting progress: {error}");
        }
    }

    fn updated(&self, id: i64) {
        if let Err(error) = (MeetingsUpdated { id }).emit(&self.0) {
            error!("Failed to emit a meeting update: {error}");
        }
    }
}

/// Runs the queue for the life of the app.
fn spawn_processing(
    app: &AppHandle,
    store: Arc<MeetingsStore>,
    root: PathBuf,
) -> Arc<ProcessingQueue> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let queue = Arc::new(ProcessingQueue {
        sender,
        queued: Mutex::default(),
        current: Mutex::default(),
    });
    let worker = Arc::clone(&queue);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut operations = 0;
        while let Some(id) = receiver.recv().await {
            // Turned off: the meeting waits, and is queued again when
            // meetings are turned back on.
            if !get_settings(&app).meetings_enabled {
                worker
                    .queued
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                continue;
            }
            operations += 1;
            let operation = ProcessingOperation::new(OperationId(operations));
            worker
                .queued
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id);
            *worker
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((id, operation.clone()));
            let language = get_settings(&app).selected_language;
            let processor = Processor {
                store: Arc::clone(&store),
                root: root.clone(),
                transcriber: Arc::new(CodexTranscriber {
                    url: crate::codex_transcribe::TRANSCRIBE_URL.to_string(),
                    auth_path: None,
                }),
                policy: RetryPolicy::default(),
                language: Some(language),
                events: Arc::new(AppEvents(app.clone())),
            };
            info!("Meeting {id}: processing");
            if let Err(error) = processor.process(id, operation).await {
                error!("Meeting {id}: processing failed: {error}");
                if let Err(error) = store.finish_processing(id, Some(ProcessingError::Other)) {
                    error!("Meeting {id}: cannot record the failure: {error}");
                }
                AppEvents(app.clone()).updated(id);
            }
            *worker
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    });
    queue
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
    queue: Arc<ProcessingQueue>,
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

    fn recorded(&self, id: i64) {
        self.queue.enqueue(id);
    }
}

type AppCoordinator = Coordinator<DeviceCapture, AppNotifier, SystemClock>;

pub struct MeetingsManager {
    app: AppHandle,
    coordinator: Mutex<AppCoordinator>,
    runtime: Arc<Mutex<MeetingRuntime>>,
    store: Arc<MeetingsStore>,
    queue: Arc<ProcessingQueue>,
}

impl MeetingsManager {
    /// Opens the library, marks meetings a crash cut off as interrupted and
    /// resumes processing the ones a quit left mid-way.
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
        let queue = spawn_processing(app, Arc::clone(&store), root.clone());
        let runtime = Arc::new(Mutex::new(MeetingRuntime::Idle));
        let notifier = AppNotifier {
            app: app.clone(),
            runtime: Arc::clone(&runtime),
            queue: Arc::clone(&queue),
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
            queue,
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
        self.queue.cancel(Some(id));
        self.coordinator()
            .delete(id)
            .map_err(|error| error.to_string())
    }

    pub fn detail(&self, id: i64) -> Result<MeetingDetail, String> {
        self.store
            .get(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Meeting {id} does not exist"))
    }

    fn updated(&self, id: i64) {
        AppEvents(self.app.clone()).updated(id);
    }

    /// An empty title gives the meeting back its default name.
    pub fn rename(&self, id: i64, title: &str) -> Result<(), String> {
        let title = rename::title(title).map_err(|error| error.to_string())?;
        self.store
            .rename(id, title.as_deref())
            .map_err(|error| error.to_string())?;
        self.updated(id);
        Ok(())
    }

    pub fn rename_speaker(&self, id: i64, key: &str, name: &str) -> Result<(), String> {
        let name = rename::speaker_name(name).map_err(|error| error.to_string())?;
        self.store
            .rename_speaker(id, key, &name)
            .map_err(|error| error.to_string())?;
        self.updated(id);
        Ok(())
    }

    /// Processes an interrupted meeting, or the parts of a failed one that
    /// are missing.
    pub fn retry(&self, id: i64) {
        self.queue.enqueue(id);
    }

    /// Queues the meetings a quit (or turning meetings off) left mid-way.
    pub fn resume_processing(&self) {
        match self.store.waiting_for_processing() {
            Ok(ids) => ids.into_iter().for_each(|id| self.queue.enqueue(id)),
            Err(error) => error!("Failed to list meetings to process: {error}"),
        }
    }

    /// The transcript as Markdown, and the file name to save it under.
    pub fn markdown(&self, id: i64) -> Result<(String, String), String> {
        let meeting = self.detail(id)?;
        let started_at = chrono::Local
            .timestamp_millis_opt(meeting.started_at_ms)
            .single()
            .ok_or_else(|| format!("Meeting {id} has an invalid start time"))?
            .fixed_offset();
        let markdown = export::transcript_markdown(&export::Transcript {
            title: meeting.title.as_deref(),
            started_at,
            duration_ms: meeting.duration_ms,
            speakers: &meeting.speakers,
            turns: &meeting.turns,
        });
        Ok((
            markdown,
            export::file_name(&started_at, meeting.title.as_deref()),
        ))
    }

    /// Keeps a meeting in progress when the app quits: its tracks are closed
    /// and it waits for processing, which resumes at the next launch.
    pub fn shutdown(&self) {
        if self.is_recording() {
            if let Err(error) = self.stop() {
                error!("Failed to finish the meeting on quit: {error}");
            }
        }
        self.queue.cancel(None);
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
            if at_startup {
                manager.resume_processing();
            }
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
