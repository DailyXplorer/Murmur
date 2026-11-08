//! Meeting recording: the microphone ("Me") and system audio (the remote
//! participants) on two tracks, started and stopped by the user, kept in a
//! library of their own. Everything here stays behind `meetings_enabled`,
//! except the audio retention the user chose.

pub mod align;
pub mod analysis;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod analysis_cpp;
pub mod announce;
// The logic needs no Windows API, so macOS compiles it for its tests only.
#[cfg(any(all(target_os = "windows", target_arch = "x86_64"), test))]
#[cfg_attr(
    not(all(target_os = "windows", target_arch = "x86_64")),
    allow(dead_code)
)]
pub mod analysis_local;
pub mod archive;
pub mod bleed;
// What Windows tells about calls; its parsers and decisions are tested on
// every host, only the reads of the system are Windows-only.
#[cfg(any(target_os = "windows", test))]
pub mod call_probe_windows;
pub mod call_watch;
pub mod calls;
pub mod cap;
pub mod capture;
pub mod chunking;
pub mod coordinator;
// Speaker diarization with pyannote Community-1: every stage but the two
// networks is plain Rust and tested on any host; only the ONNX Runtime glue
// (`diarize::pyannote`) is Windows-only.
#[cfg_attr(
    not(all(target_os = "windows", target_arch = "x86_64")),
    allow(dead_code)
)]
pub mod diarize;
pub mod export;
pub mod helper;
pub mod import_audio;
pub mod live;
pub mod merge;
pub mod models;
pub mod offline;
pub mod open_panel;
pub mod paths;
#[cfg(target_os = "macos")]
pub mod power;
// Kept apart from `power.rs` so that file stays what `main` has.
#[cfg(target_os = "windows")]
#[path = "power_windows.rs"]
pub mod power;
pub mod processor;
pub mod protocol;
pub mod rename;
pub mod retention;
pub mod save_panel;
// Kills its child processes (SIGKILL on a Mac, TerminateProcess on Windows)
// and reads each platform's process memory. The three-hour run with `say` and
// the helper is macOS-only; Windows has a soak of its own in the same file.
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod soak;
pub mod speakers;
pub mod spread;
pub mod store;
pub mod summary;
pub mod tokens;
pub mod track_audio;
pub mod transcribe;
pub mod trash;
// The Recycle Bin call is Windows-only; what decides its input and result is
// plain code, tested on every host.
#[cfg(any(target_os = "windows", test))]
mod trash_windows;
pub mod wav_recovery;

mod audio_retention;
mod manager;
mod processing;
mod queue;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use log::{error, info, warn};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::local_models::LocalModelState;
use crate::managers::history::DatabaseStartup;
use crate::settings::{get_settings, AppSettings, MeetingAudioRetention};
use crate::user_alert::{self, Alert};
use crate::{OperationId, ProcessingOperation};
use audio_retention::*;
use capture::system::SystemAudioSupport;
use capture::DeviceCapture;
use coordinator::{Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier};
use helper::AnalysisSupport;
use import_audio::{ImportError, MeetingImport, MeetingImportChanged, Stopped};
use models::Models;
use offline::{CodexHost, OfflineHold, Reachability};
use processing::*;
use processor::{MeetingProgress, ProcessEvents, Processor};
use queue::*;
use store::{MeetingDetail, MeetingHit, MeetingsPage, MeetingsStore, ProcessingError};
use summary::CodexSummarizer;
use transcribe::{CodexTranscriber, CodexVerdict, LocalTranscriber, RetryPolicy, Transcriber};

pub const BINDING_ID: &str = "meeting_toggle";
/// Option+Command+M is macOS's "Minimize All"; adding Control frees it.
#[cfg(target_os = "macos")]
pub const DEFAULT_BINDING: &str = "ctrl+option+command+m";
/// The default of the betas, where Meetings were hidden. Ctrl+Alt+Win+M reads
/// as AltGr+Win+M on layouts that have an AltGr key.
#[cfg(target_os = "windows")]
pub const PREVIOUS_DEFAULT_BINDING: &str = "ctrl+option+command+m";
/// Ctrl+Alt is AltGr on many layouts and Win+Ctrl+M is taken by Windows
/// itself: Alt+Shift+M is left to Murmur, as Alt+Shift+V is for paste last.
#[cfg(target_os = "windows")]
pub const DEFAULT_BINDING: &str = "alt+shift+m";
pub const TRAY_ITEM_ID: &str = "meeting_toggle";

/// Sent whenever a meeting starts or stops.
#[derive(Serialize, Debug, Clone, Type, Event)]
pub struct MeetingStateChanged(pub MeetingRuntime);

/// A meeting's row or transcript changed: lists and open views reload it.
#[derive(Serialize, Debug, Clone, Copy, Type, Event)]
pub struct MeetingsUpdated {
    pub id: i64,
}

/// Which tab Copy and Export take their text from.
#[derive(Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingMarkdownKind {
    Summary,
    Transcript,
}

#[derive(Serialize, Debug, Clone, Copy, Type)]
pub struct MeetingCapabilities {
    pub system_audio: SystemAudioSupport,
    /// Telling speakers apart on device.
    pub analysis: AnalysisSupport,
    /// Telling that a call started, for the call prompt.
    pub call_detection: bool,
}

/// Recorded meetings wait here and are processed one at a time, in order.
/// After a rate-limited meeting, or one that could not reach Codex, the
/// queue holds: only meetings the user asked for go through, ahead of the
/// others. While only the connection holds it and this Mac's model is
/// ready, the meetings that model can still transcribe go through too.
struct ProcessingQueue {
    state: Mutex<QueueState>,
    changed: tokio::sync::Notify,
    reachability: Box<dyn Reachability>,
    /// Whether this Mac's model is ready.
    local_ready: Box<dyn Fn() -> bool + Send + Sync>,
}

struct QueueState {
    queued: VecDeque<Queued>,
    current: Option<(i64, ProcessingOperation)>,
    paused: bool,
    /// A rate-limit hold: until then, only `asked` meetings go through. Wall
    /// time, as Codex counts its cap: `Instant` stops while the Mac sleeps.
    held_until: Option<SystemTime>,
    rate_limit: RateLimitHold,
    /// Holds the queue, as the rate-limit hold does, until the connection
    /// is back.
    offline: OfflineHold,
    /// The meeting taken last went past the offline hold on this Mac's
    /// model.
    local_pass: bool,
}

/// What the worker does next.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    Process(i64),
    /// See whether the connection is back.
    CheckConnection,
    /// Until a hold ends (`None`: nothing to wait for but a new meeting).
    Wait(Option<SystemTime>),
}

struct Queued {
    id: i64,
    /// Queued at the user's request (Retry, Transcribe again...), which a
    /// rate-limit hold lets through: pressing it is how the user says Codex
    /// may have freed up.
    asked: bool,
    /// Only Codex can do what is left: a run had this Mac's model, or the
    /// transcript is complete and only the summary is left.
    codex_only: bool,
}

/// How long the queue waits after Codex rate limits a meeting's transcript
/// or summary, before the next one: Codex would refuse it too, and each
/// meeting would spend its half hour of retries and fail. The ChatGPT usage
/// cap frees up over a sliding five-hour window, so each refusal in a row,
/// with no answer between, doubles the wait.
#[derive(Default)]
struct RateLimitHold {
    in_a_row: u32,
}

struct AppEvents(AppHandle);

struct SystemClock;

/// Shows the recording state everywhere it appears. The coordinator calls
/// it with its lock held, so nothing here waits on the main thread.
struct AppNotifier {
    app: AppHandle,
    runtime: Arc<Mutex<MeetingRuntime>>,
    queue: Arc<ProcessingQueue>,
}

type AppCoordinator = Coordinator<DeviceCapture, AppNotifier, SystemClock>;

/// The meetings library as a launch finds it.
struct Library {
    store: MeetingsStore,
    root: PathBuf,
    /// Meetings a crash cut off before this launch, left to process.
    recovered: Vec<i64>,
    /// The database was damaged or gone, and the meetings it listed were
    /// set aside, to be deleted 30 days later.
    reset: bool,
}

pub struct MeetingsManager {
    app: AppHandle,
    coordinator: Mutex<AppCoordinator>,
    runtime: Arc<Mutex<MeetingRuntime>>,
    store: Arc<MeetingsStore>,
    queue: Arc<ProcessingQueue>,
    root: PathBuf,
    models: Arc<Models>,
    /// Meetings a crash cut off before this launch, left to process.
    recovered: Vec<i64>,
    /// The audio file being imported, if any: one at a time.
    import: Mutex<Option<ActiveImport>>,
}

/// An import under way: what the page shows, and how to stop it.
struct ActiveImport {
    cancel: Arc<AtomicBool>,
    shown: MeetingImport,
}

/// Frees the import slot when the import ends, however it ends.
struct ImportSlot<'a>(&'a MeetingsManager);

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
            // Meetings turned off keep the interrupted ones until the user
            // processes them, or turns meetings back on.
            if at_startup && get_settings(app).meetings_enabled {
                manager.resume_processing();
                // One notification, however many a crash cut off.
                let count = manager.recovered.len();
                if count > 0 {
                    user_alert::show(app, Alert::MeetingsRecovered { count });
                }
            }
            app.manage(Arc::new(manager));
            spawn_audio_retention(app);
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
mod tests;
