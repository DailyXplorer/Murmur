//! Meeting recording: the microphone ("Me") and system audio (the remote
//! participants) on two tracks, started and stopped by the user, kept in a
//! library of their own. Everything here stays behind `meetings_enabled`.

pub mod align;
pub mod analysis;
pub mod archive;
pub mod bleed;
pub mod call_watch;
pub mod calls;
pub mod cap;
pub mod capture;
pub mod chunking;
pub mod coordinator;
pub mod export;
pub mod helper;
pub mod merge;
pub mod models;
pub mod offline;
pub mod paths;
pub mod power;
pub mod processor;
pub mod protocol;
pub mod rename;
pub mod retention;
pub mod save_panel;
#[cfg(test)]
mod soak;
pub mod speakers;
pub mod spread;
pub mod store;
pub mod summary;
pub mod tokens;
pub mod track_audio;
pub mod transcribe;
pub mod wav_recovery;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::TimeZone;
use log::{error, info, warn};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::local_models::LocalModelState;
use crate::managers::history::DatabaseStartup;
use crate::settings::{get_settings, MeetingAudioRetention};
use crate::user_alert::{self, Alert};
use crate::{OperationId, ProcessingOperation};
use analysis::HelperAnalyzer;
use capture::system::SystemAudioSupport;
use capture::DeviceCapture;
use coordinator::{Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier};
use helper::AnalysisSupport;
use models::Models;
use offline::{CodexHost, OfflineHold, Reachability};
use processor::{MeetingProgress, ProcessEvents, Processor};
use store::{MeetingDetail, MeetingsPage, MeetingsStore, ProcessingError};
use summary::CodexSummarizer;
use transcribe::{CodexTranscriber, CodexVerdict, LocalTranscriber, RetryPolicy, Transcriber};

pub const BINDING_ID: &str = "meeting_toggle";
/// Option+Command+M is macOS's "Minimize All"; adding Control frees it.
pub const DEFAULT_BINDING: &str = "ctrl+option+command+m";
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

impl QueueState {
    fn held(&self) -> bool {
        self.held_until.is_some() || self.offline.holds()
    }

    fn goes_through(&self, queued: &Queued, local_ready: bool) -> bool {
        !self.held()
            || queued.asked
            || (self.held_until.is_none() && local_ready && !queued.codex_only)
    }

    /// The meeting to process now or, when none may go, what to wait for.
    fn take_next(&mut self, now: SystemTime, local_ready: bool) -> Next {
        self.held_until = self.held_until.filter(|until| now < *until);
        let index = self
            .queued
            .iter()
            .position(|queued| self.goes_through(queued, local_ready));
        if let Some(queued) = index.and_then(|index| self.queued.remove(index)) {
            self.local_pass = self.held() && !queued.asked;
            return Next::Process(queued.id);
        }
        if self.offline.due(now) {
            return Next::CheckConnection;
        }
        let ends = [self.held_until, self.offline.retry_at()];
        Next::Wait(ends.into_iter().flatten().min())
    }
}

impl ProcessingQueue {
    fn new(paused: bool) -> Self {
        let state = QueueState {
            queued: VecDeque::new(),
            current: None,
            paused,
            held_until: None,
            rate_limit: RateLimitHold::default(),
            offline: OfflineHold::default(),
            local_pass: false,
        };
        Self {
            state: Mutex::new(state),
            changed: tokio::sync::Notify::new(),
            reachability: Box::new(CodexHost::new()),
            local_ready: Box::new(|| false),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn enqueue(&self, id: i64) {
        self.push(id, false, false);
    }

    /// Queues a meeting the user just asked for, which a rate-limit hold
    /// does not keep waiting. Already queued, it keeps its place in line
    /// and goes through the hold.
    fn enqueue_asked(&self, id: i64) {
        self.push(id, true, false);
    }

    /// Queues a meeting that an offline hold may keep waiting for the
    /// connection. `codex_only`: this Mac's model cannot do what is left.
    fn requeue(&self, id: i64, codex_only: bool) {
        self.push(id, false, codex_only);
    }

    /// Meetings waiting for the connection may go on with this Mac's model.
    fn local_model_ready(&self) {
        self.changed.notify_one();
    }

    fn push(&self, id: i64, asked: bool, codex_only: bool) {
        {
            let mut state = self.state();
            match state.queued.iter_mut().find(|queued| queued.id == id) {
                Some(queued) => {
                    queued.asked |= asked;
                    queued.codex_only |= codex_only;
                }
                None => state.queued.push_back(Queued {
                    id,
                    asked,
                    codex_only,
                }),
            }
        }
        // `notify_one` keeps a permit, so a worker between its check and its
        // wait still wakes; a stale permit only costs it one more check.
        self.changed.notify_one();
    }

    /// How often a held queue looks at the clock again: timers run on uptime,
    /// so after a sleep only a fresh look sees that the hold has ended.
    const RECHECK: Duration = Duration::from_secs(60);

    /// Waits for the next meeting to process, past any rate-limit hold and
    /// until the connection is back after Codex could not be reached.
    async fn next(&self) -> i64 {
        loop {
            let changed = self.changed.notified();
            let now = SystemTime::now();
            let local_ready = (self.local_ready)();
            let next = self.state().take_next(now, local_ready);
            match next {
                Next::Process(id) => return id,
                Next::CheckConnection => {
                    let online = self.reachability.online().await;
                    if online {
                        info!("The connection is back; meetings waiting for it go on");
                    }
                    self.state().offline.checked(online, SystemTime::now());
                }
                Next::Wait(Some(until)) => {
                    let left = until.duration_since(now).unwrap_or_default();
                    tokio::select! {
                        () = tokio::time::sleep(left.min(Self::RECHECK)) => {}
                        () = changed => {}
                    }
                }
                Next::Wait(None) => changed.await,
            }
        }
    }

    /// Holds the meetings not asked for during `wait`.
    #[cfg(test)]
    fn hold(&self, wait: Duration) {
        self.state().held_until = Some(SystemTime::now() + wait);
    }

    /// Holds or frees the queue from what Codex said while a meeting was
    /// processed, and returns the wait it starts. Only an answer lifts a hold: a
    /// pass that asked Codex nothing, such as re-analysing, says nothing
    /// about its cap.
    fn codex_said(&self, verdict: CodexVerdict) -> Option<Duration> {
        let mut state = self.state();
        let local_pass = std::mem::take(&mut state.local_pass);
        match verdict {
            CodexVerdict::RateLimited => {
                // A refusal is an answer: the connection works.
                state.offline.reset();
                let wait = state.rate_limit.rate_limited();
                state.held_until = Some(SystemTime::now() + wait);
                Some(wait)
            }
            CodexVerdict::Unreachable => {
                let now = SystemTime::now();
                match state.offline.retry_at() {
                    // The hold already waits for the connection: a meeting
                    // this Mac's model took past it leaves the next check
                    // where it was.
                    Some(at) if local_pass => Some(at.duration_since(now).unwrap_or_default()),
                    _ => Some(state.offline.unreachable(now)),
                }
            }
            CodexVerdict::Answered => {
                state.offline.reset();
                state.rate_limit.reset();
                state.held_until = None;
                None
            }
            CodexVerdict::Unknown => None,
        }
    }

    /// Makes `id` the meeting in progress, unless the queue is paused: the
    /// meeting then waits, and `resume_processing` queues it again. Deciding
    /// under the lock `pause` takes means a meeting is either refused here
    /// or cancelled there, never sent after a pause.
    fn begin(&self, id: i64, operation: &ProcessingOperation) -> bool {
        let mut state = self.state();
        if state.paused {
            return false;
        }
        state.current = Some((id, operation.clone()));
        true
    }

    fn end(&self) {
        self.state().current = None;
    }

    /// How much longer a rate-limit hold keeps queued meeting `id` waiting.
    /// `None` when it is not waiting on one.
    fn held_for(&self, id: i64) -> Option<Duration> {
        let state = self.state();
        let left = state.held_until?.duration_since(SystemTime::now()).ok()?;
        state
            .queued
            .iter()
            .any(|queued| queued.id == id && !queued.asked)
            .then_some(left)
    }

    /// Whether queued meeting `id` waits for the connection to come back.
    fn offline_for(&self, id: i64) -> bool {
        let local_ready = (self.local_ready)();
        let state = self.state();
        state.offline.holds()
            && state
                .queued
                .iter()
                .any(|queued| queued.id == id && !state.goes_through(queued, local_ready))
    }

    fn queued(&self) -> Vec<i64> {
        self.state().queued.iter().map(|queued| queued.id).collect()
    }

    /// The meetings queued or in progress, which may still read their audio.
    fn busy(&self) -> HashSet<i64> {
        let state = self.state();
        state
            .queued
            .iter()
            .map(|queued| queued.id)
            .chain(state.current.as_ref().map(|(id, _)| *id))
            .collect()
    }

    /// Stops processing meeting `id`. What was saved stays, and processing
    /// resumes from there next time.
    fn cancel(&self, id: i64) {
        if let Some((current, operation)) = &self.state().current {
            if *current == id {
                operation.cancel();
            }
        }
    }

    /// Stops the meeting in progress and holds every other one until
    /// `resume`.
    fn pause(&self) {
        let mut state = self.state();
        state.paused = true;
        if let Some((_, operation)) = &state.current {
            operation.cancel();
        }
    }

    fn resume(&self) {
        self.state().paused = false;
    }
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

impl RateLimitHold {
    const FIRST: Duration = Duration::from_secs(30 * 60);
    const CAP: Duration = Duration::from_secs(4 * 3600);

    /// The wait after one more refusal.
    fn rate_limited(&mut self) -> Duration {
        let wait = Self::FIRST
            .saturating_mul(1 << self.in_a_row.min(8))
            .min(Self::CAP);
        self.in_a_row += 1;
        wait
    }

    /// Codex answered: the cap has freed up.
    fn reset(&mut self) {
        self.in_a_row = 0;
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
    models: Arc<Models>,
) -> Arc<ProcessingQueue> {
    let mut queue = ProcessingQueue::new(!get_settings(app).meetings_enabled);
    let ready = Arc::clone(&models);
    queue.local_ready = Box::new(move || ready.is_ready());
    let queue = Arc::new(queue);
    let mut model_state = models.subscribe();
    let waking = Arc::clone(&queue);
    tauri::async_runtime::spawn(async move {
        while model_state.changed().await.is_ok() {
            if *model_state.borrow_and_update() == LocalModelState::Ready {
                waking.local_model_ready();
            }
        }
    });
    let worker = Arc::clone(&queue);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut operations = 0;
        loop {
            let id = worker.next().await;
            operations += 1;
            let operation = ProcessingOperation::new(OperationId(operations));
            if !worker.begin(id, &operation) {
                continue;
            }
            // Its view stops saying it waits for a hold or the connection.
            AppEvents(app.clone()).updated(id);
            let language = get_settings(&app).selected_language;
            let local_tried = models.is_ready();
            let processor = Processor {
                store: Arc::clone(&store),
                root: root.clone(),
                transcriber: Arc::new(CodexTranscriber {
                    url: crate::codex_transcribe::TRANSCRIBE_URL.to_string(),
                    auth_path: None,
                }),
                // Its own engine, released with the run.
                local: local_tried
                    .then(|| crate::engine::local::engine(&app, Box::new(|| false)))
                    .flatten()
                    .map(|engine| {
                        Arc::new(LocalTranscriber(Arc::new(engine))) as Arc<dyn Transcriber>
                    }),
                analyzer: Arc::new(HelperAnalyzer {
                    models: Arc::clone(&models),
                    support: helper::support(),
                }),
                summarizer: Arc::new(CodexSummarizer {
                    url: crate::cleanup::responses::RESPONSES_URL.to_string(),
                    auth_path: None,
                }),
                policy: RetryPolicy::default(),
                language: Some(language),
                events: Arc::new(AppEvents(app.clone())),
            };
            info!("Meeting {id}: processing");
            let codex = match processor.process(id, operation).await {
                Ok(codex) => codex,
                Err(error) => {
                    error!("Meeting {id}: processing failed: {error}");
                    if let Err(error) = store.finish_processing(id, Some(ProcessingError::Other)) {
                        error!("Meeting {id}: cannot record the failure: {error}");
                    }
                    AppEvents(app.clone()).updated(id);
                    CodexVerdict::Unknown
                }
            };
            worker.end();
            // While held, the meetings still queued wait as they are,
            // unfailed.
            match (codex, worker.codex_said(codex)) {
                (CodexVerdict::Unreachable, Some(wait)) => {
                    info!(
                        "Meeting {id}: Codex is out of reach; checking the connection again in {wait:?}"
                    );
                    // It goes on with the others once the connection is back.
                    worker.requeue(id, local_tried || codex_only(&store, id));
                }
                (_, Some(wait)) => {
                    info!("Meeting {id}: Codex is rate limiting; holding the queue for {wait:?}");
                }
                (_, None) => {}
            }
            // A hold that starts or lifts changes what their views say.
            if codex != CodexVerdict::Unknown {
                let events = AppEvents(app.clone());
                worker
                    .queued()
                    .into_iter()
                    .for_each(|id| events.updated(id));
            }
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
        crate::utils::update_tray_menu(&self.app, None);
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

/// Queues the meetings a quit (or turning meetings off) left mid-way.
fn queue_waiting(queue: &ProcessingQueue, store: &MeetingsStore) {
    match store.waiting_for_processing() {
        Ok(ids) => ids
            .into_iter()
            .for_each(|id| queue.requeue(id, codex_only(store, id))),
        Err(error) => error!("Failed to list meetings to process: {error}"),
    }
}

/// Meeting `id` waits for its summary only, which this Mac's model cannot
/// write.
fn codex_only(store: &MeetingsStore, id: i64) -> bool {
    store.get(id).ok().flatten().is_some_and(|meeting| {
        meeting.status == store::MeetingStatus::Ready
            && meeting.summary_status == store::SummaryStatus::Pending
    })
}

/// Deletes finished meeting `id`'s audio and the analysis made from it.
fn delete_audio(store: &MeetingsStore, root: &Path, id: i64) -> Result<(), String> {
    if !store.delete_audio(id).map_err(|error| error.to_string())? {
        return Err(format!("Meeting {id} is not finished"));
    }
    paths::remove_meeting_files(root, id, &paths::AUDIO_FILES)
        .and_then(|()| paths::remove_meeting_files(root, id, &[analysis::ANALYSIS_FILE]))
        .map_err(|error| format!("Cannot delete the meeting audio: {error}"))
}

/// Deletes the audio `retention` has kept long enough at `now_ms`, except
/// that of the `busy` meetings, and returns whose went.
fn sweep_audio(
    store: &MeetingsStore,
    root: &Path,
    retention: MeetingAudioRetention,
    now_ms: i64,
    busy: &HashSet<i64>,
) -> Vec<i64> {
    let meetings = match store.with_audio() {
        Ok(meetings) => meetings,
        Err(error) => {
            error!("Failed to list the meetings that have audio: {error}");
            return Vec::new();
        }
    };
    retention::audio_to_delete(&meetings, now_ms, retention)
        .into_iter()
        .filter(|id| !busy.contains(id))
        .filter(|id| match delete_audio(store, root, *id) {
            Ok(()) => true,
            Err(error) => {
                error!("Meeting {id}: {error}");
                false
            }
        })
        .collect()
}

/// Applies the audio retention at launch, then once a day. The hourly look
/// reads the wall clock, which goes on while the Mac sleeps.
fn spawn_audio_retention(app: &AppHandle) {
    const DAY: Duration = Duration::from_secs(24 * 3600);
    const LOOK: Duration = Duration::from_secs(3600);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: Option<SystemTime> = None;
        loop {
            let due = last.is_none_or(|last| {
                SystemTime::now()
                    .duration_since(last)
                    .map_or(true, |since| since >= DAY)
            });
            if due && get_settings(&app).meetings_enabled {
                if let Some(manager) = app.try_state::<Arc<MeetingsManager>>() {
                    let manager = Arc::clone(&manager);
                    let swept = tauri::async_runtime::spawn_blocking(move || {
                        manager.apply_audio_retention()
                    });
                    if let Err(error) = swept.await {
                        error!("Meeting audio retention failed: {error}");
                    }
                    last = Some(SystemTime::now());
                }
            }
            tokio::time::sleep(LOOK).await;
        }
    });
}

/// The meetings library as a launch finds it.
struct Library {
    store: MeetingsStore,
    root: PathBuf,
    /// Meetings a crash cut off before this launch, left to process.
    recovered: Vec<i64>,
}

impl Library {
    /// Opens the library in `app_data`, sets aside what no meeting points
    /// to, and marks meetings a crash cut off as interrupted.
    fn open(app_data: &Path) -> anyhow::Result<Self> {
        let root = app_data.join("meetings");
        std::fs::create_dir_all(&root)?;
        let now = chrono::Utc::now();
        let (store, startup) = MeetingsStore::open(app_data.join("meetings.db"), now);
        info!("Meetings database: {startup:?}");
        match &startup {
            DatabaseStartup::Reset { moved_to } => {
                if let Err(error) = paths::set_aside_recordings(&root, moved_to) {
                    error!("Failed to set aside the recordings of the reset library: {error}");
                }
            }
            // Only a library that opened says which folders no row points to.
            DatabaseStartup::Ready => {
                match paths::remove_stale(&root) {
                    0 => {}
                    count => info!("Removed {count} stale meeting folder(s)"),
                }
                let aside =
                    app_data.join(format!("meetings.orphaned-{}", now.format("%Y%m%d-%H%M%S")));
                let moved = store
                    .ids()
                    .and_then(|ids| Ok(paths::set_aside_orphans(&root, &ids, &aside)?));
                match moved {
                    Ok(0) => {}
                    Ok(count) => warn!(
                        "Moved {count} meeting folder(s) no meeting points to into {}",
                        aside.display()
                    ),
                    Err(error) => error!("Failed to set aside orphan meeting folders: {error}"),
                }
            }
            DatabaseStartup::Unavailable => {}
        }
        match paths::purge_set_asides(app_data, now) {
            0 => {}
            count => {
                info!("Deleted {count} meeting folder(s) or file(s) set aside over 30 days ago")
            }
        }
        let recovered = store
            .recover_interrupted(&root)
            .inspect_err(|error| error!("Failed to recover interrupted meetings: {error}"))
            .unwrap_or_default();
        if !recovered.is_empty() {
            info!("Recovered interrupted meeting(s) {recovered:?}");
        }
        Ok(Self {
            store,
            root,
            recovered,
        })
    }
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
}

impl MeetingsManager {
    /// Opens the library and marks meetings a crash cut off as interrupted.
    pub fn new(app: &AppHandle) -> anyhow::Result<Self> {
        let Library {
            store,
            root,
            recovered,
        } = Library::open(&app.path().app_data_dir()?)?;
        let store = Arc::new(store);
        let models = app
            .try_state::<Arc<Models>>()
            .map(|models| Arc::clone(&models))
            .ok_or_else(|| anyhow::anyhow!("The local models are unavailable"))?;
        let queue = spawn_processing(app, Arc::clone(&store), root.clone(), Arc::clone(&models));
        let runtime = Arc::new(Mutex::new(MeetingRuntime::Idle));
        let notifier = AppNotifier {
            app: app.clone(),
            runtime: Arc::clone(&runtime),
            queue: Arc::clone(&queue),
        };
        Ok(Self {
            app: app.clone(),
            coordinator: Mutex::new(Coordinator::new(
                DeviceCapture(app.clone()),
                notifier,
                SystemClock,
                Arc::clone(&store),
                root.clone(),
            )),
            runtime,
            store,
            queue,
            root,
            models,
            recovered,
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
        // Opening the devices takes a moment. Meetings turned off meanwhile
        // found nothing recording to stop, and left no way to stop this one.
        if !get_settings(&self.app).meetings_enabled {
            let _ = coordinator.discard(id);
            return Err("Meetings were turned off while the meeting started".to_string());
        }
        info!("Meeting {id} started");
        self.spawn_ticker(id);
        // The speaker models download while the meeting records, so they are
        // ready when it stops. Intel Macs tell no speakers apart.
        if helper::support() == AnalysisSupport::Available {
            self.models.download_unless_declined();
        }
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

    pub fn discard(&self, id: i64) -> Result<(), String> {
        self.coordinator()
            .discard(id)
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
        self.queue.cancel(id);
        self.coordinator()
            .delete(id)
            .map_err(|error| error.to_string())?;
        self.updated(id);
        Ok(())
    }

    pub fn detail(&self, id: i64) -> Result<MeetingDetail, String> {
        let mut meeting = self
            .store
            .get(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Meeting {id} does not exist"))?;
        meeting.held_until_ms = self.queue.held_for(id).map(|left| {
            chrono::Utc::now().timestamp_millis() + i64::try_from(left.as_millis()).unwrap_or(0)
        });
        meeting.waiting_for_connection = self.queue.offline_for(id);
        Ok(meeting)
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
    /// are missing. It shows as waiting at once, even while another meeting
    /// is processing.
    pub fn retry(&self, id: i64) {
        match self.store.wait_for_retry(id) {
            Ok(true) => self.updated(id),
            Ok(false) => {}
            Err(error) => error!("Meeting {id}: cannot mark it waiting: {error}"),
        }
        self.queue.enqueue_asked(id);
    }

    /// Tells the speakers of a finished meeting apart again, with the models
    /// as they are now. Codex is not asked again for parts it transcribed.
    pub fn reanalyze(&self, id: i64) -> Result<(), String> {
        if self.detail(id)?.audio == store::AudioState::Deleted {
            return Err(format!("Meeting {id} has no audio left"));
        }
        if !self.store.reopen(id).map_err(|error| error.to_string())? {
            return Err(format!("Meeting {id} is not finished"));
        }
        // A summary still being written is of the analysis that goes.
        self.queue.cancel(id);
        // Nothing processes the meeting until it is queued below.
        let analysis = paths::meeting_dir(&self.root, id).join(analysis::ANALYSIS_FILE);
        match std::fs::remove_file(analysis) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => error!("Meeting {id}: cannot reset the analysis: {error}"),
        }
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Asks Codex again for a summary that failed.
    pub fn retry_summary(&self, id: i64) -> Result<(), String> {
        if !self
            .store
            .retry_summary(id)
            .map_err(|error| error.to_string())?
        {
            return Err(format!("Meeting {id} has no failed summary"));
        }
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Deletes a finished meeting's audio and the analysis made from it; the
    /// transcript and the summary stay.
    pub fn delete_audio(&self, id: i64) -> Result<(), String> {
        let result = delete_audio(&self.store, &self.root, id);
        self.updated(id);
        result
    }

    /// Deletes the audio the retention the user chose has kept long enough.
    /// A meeting queued or processing keeps it until it is done.
    fn apply_audio_retention(&self) {
        let retention = get_settings(&self.app).meeting_audio_retention;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let deleted = sweep_audio(
            &self.store,
            &self.root,
            retention,
            now_ms,
            &self.queue.busy(),
        );
        if !deleted.is_empty() {
            info!("Deleted the audio of meeting(s) {deleted:?}, kept as long as chosen");
        }
        deleted.into_iter().for_each(|id| self.updated(id));
    }

    /// Transcribes a finished meeting again from its audio, and writes its
    /// summary anew. A title the user gave stays.
    pub fn retranscribe(&self, id: i64) -> Result<(), String> {
        if !self
            .store
            .retranscribe(id)
            .map_err(|error| error.to_string())?
        {
            return Err(format!("Meeting {id} cannot be transcribed again"));
        }
        // A summary still being written is of the transcript that goes.
        self.queue.cancel(id);
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Lets processing run again and queues the meetings a quit (or turning
    /// meetings off) left mid-way.
    pub fn resume_processing(&self) {
        self.queue.resume();
        queue_waiting(&self.queue, &self.store);
    }

    /// The summary or the transcript as Markdown, and the file name to save
    /// it under.
    pub fn markdown(&self, id: i64, kind: MeetingMarkdownKind) -> Result<(String, String), String> {
        let meeting = self.detail(id)?;
        let started_at = chrono::Local
            .timestamp_millis_opt(meeting.started_at_ms)
            .single()
            .ok_or_else(|| format!("Meeting {id} has an invalid start time"))?
            .fixed_offset();
        let transcript = export::Transcript {
            title: meeting.title.as_deref(),
            started_at,
            duration_ms: meeting.duration_ms,
            speakers: &meeting.speakers,
            turns: &meeting.turns,
        };
        let markdown = match kind {
            MeetingMarkdownKind::Transcript => export::transcript_markdown(&transcript),
            MeetingMarkdownKind::Summary => export::summary_markdown(
                &transcript,
                meeting
                    .summary
                    .as_deref()
                    .ok_or_else(|| format!("Meeting {id} has no summary"))?,
            ),
        };
        Ok((
            markdown,
            export::file_name(&started_at, meeting.title.as_deref()),
        ))
    }

    /// Keeps a meeting in progress when the app quits or meetings are turned
    /// off: its tracks are closed and it waits for processing, which resumes
    /// at the next launch or when meetings are turned back on.
    pub fn shutdown(&self) {
        if self.is_recording() {
            if let Err(error) = self.stop() {
                error!("Failed to finish the meeting on quit: {error}");
            }
        }
        // A models download goes on: dictation shares the models.
        self.queue.pause();
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
mod tests {
    use super::*;

    #[test]
    fn default_binding_is_a_valid_shortcut() {
        crate::shortcut::tauri_impl::parse_shortcut(DEFAULT_BINDING).unwrap();
    }

    #[test]
    fn default_binding_leaves_standard_macos_shortcuts_alone() {
        // A global hotkey swallows its keys in every app, so the default must
        // not be one macOS apps already use: Option+Command+M minimizes all
        // windows, Command+M minimizes one, Control+Command+Q locks the screen.
        let default = crate::shortcut::tauri_impl::parse_shortcut(DEFAULT_BINDING).unwrap();
        for taken in [
            "command+m",
            "option+command+m",
            "ctrl+command+m",
            "command+h",
            "option+command+h",
            "ctrl+command+q",
            "ctrl+command+f",
            "ctrl+command+space",
        ] {
            let taken = crate::shortcut::tauri_impl::parse_shortcut(taken).unwrap();
            assert_ne!(default, taken);
        }
    }

    /// A processed meeting with a transcript, a summary and compressed audio,
    /// started `days_ago` days before `now_ms`.
    fn processed(store: &MeetingsStore, root: &Path, now_ms: i64, days_ago: i64) -> i64 {
        let started = now_ms - days_ago * 24 * 3600 * 1000;
        let id = store
            .insert_recording(started, store::MeetingTracks::Mic)
            .unwrap();
        store
            .finish_recording(id, started + 60_000, 60_000)
            .unwrap();
        let me = store::MeetingSpeaker {
            key: "me".into(),
            name: "Me".into(),
            default_name: "Me".into(),
            color_index: 0,
        };
        let turn = store::MeetingTurn {
            speaker_key: "me".into(),
            start_ms: 0,
            end_ms: 1_000,
            text: "Kept.".into(),
            low_confidence: false,
        };
        store
            .save_transcript(
                id,
                &[me],
                &[turn],
                store::SpeakerAnalysis::NoModels,
                None,
                None,
            )
            .unwrap();
        store.finish_processing(id, None).unwrap();
        assert!(store.save_summary(id, None, "Notes.").unwrap());
        assert!(store.audio_compressed(id, 10).unwrap());
        let dir = paths::fresh_meeting_dir(root, id, now_ms).unwrap();
        std::fs::write(dir.join(paths::AUDIO_FILE), b"audio").unwrap();
        std::fs::write(dir.join(analysis::ANALYSIS_FILE), b"{}").unwrap();
        id
    }

    #[test]
    fn retention_deletes_old_audio_and_keeps_the_text_and_busy_meetings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("meetings");
        let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
        let now_ms = chrono::Utc::now().timestamp_millis();
        let old = processed(&store, &root, now_ms, 8);
        let recent = processed(&store, &root, now_ms, 6);
        let queued = processed(&store, &root, now_ms, 8);

        let swept =
            |retention| sweep_audio(&store, &root, retention, now_ms, &HashSet::from([queued]));
        assert!(swept(MeetingAudioRetention::Keep).is_empty());
        assert_eq!(swept(MeetingAudioRetention::Days7), [old]);
        assert!(swept(MeetingAudioRetention::Days7).is_empty());

        let meeting = store.get(old).unwrap().unwrap();
        assert_eq!(meeting.audio, store::AudioState::Deleted);
        assert_eq!(meeting.summary.as_deref(), Some("Notes."));
        assert_eq!(meeting.turns.len(), 1);
        let files = paths::meeting_dir(&root, old);
        assert!(!files.join(paths::AUDIO_FILE).exists());
        assert!(!files.join(analysis::ANALYSIS_FILE).exists());
        for kept in [recent, queued] {
            let meeting = store.get(kept).unwrap().unwrap();
            assert_eq!(meeting.audio, store::AudioState::Compressed);
            let files = paths::meeting_dir(&root, kept);
            assert!(files.join(paths::AUDIO_FILE).exists());
        }
    }

    /// Meetings set aside at launch are deleted 30 days later, so the user
    /// is told the library was reset.
    #[test]
    fn a_library_reset_at_launch_is_told() {
        let damaged = tempfile::tempdir().unwrap();
        std::fs::write(damaged.path().join("meetings.db"), b"not a database").unwrap();
        std::fs::create_dir_all(damaged.path().join("meetings").join("1")).unwrap();
        assert!(Library::open(damaged.path()).unwrap().reset, "damaged");

        let gone = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(gone.path().join("meetings").join("3")).unwrap();
        assert!(Library::open(gone.path()).unwrap().reset, "deleted");

        let first_launch = tempfile::tempdir().unwrap();
        assert!(!Library::open(first_launch.path()).unwrap().reset);
        // A folder no meeting points to, in a library that opened.
        std::fs::create_dir_all(first_launch.path().join("meetings").join("9")).unwrap();
        assert!(!Library::open(first_launch.path()).unwrap().reset);
    }

    #[test]
    fn the_meetings_queued_or_processing_are_busy() {
        let queue = ProcessingQueue::new(false);
        queue.enqueue(3);
        queue.enqueue(4);
        let id = queue.state().take_next(SystemTime::now(), false);
        assert_eq!(id, Next::Process(3));
        assert!(queue.begin(3, &operation(1)));

        assert_eq!(queue.busy(), HashSet::from([3, 4]));
        queue.end();
        assert_eq!(queue.busy(), HashSet::from([4]));
    }

    fn operation(id: u64) -> ProcessingOperation {
        ProcessingOperation::new(OperationId(id))
    }

    /// The meeting the worker would take next, or `None` when it is still
    /// waiting after `within`.
    fn next_within(queue: &ProcessingQueue, within: Duration) -> Option<i64> {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async { tokio::time::timeout(within, queue.next()).await })
            .ok()
    }

    const NOW: Duration = Duration::from_secs(5);
    const A_MOMENT: Duration = Duration::from_millis(50);
    const AN_HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn meetings_are_processed_in_the_order_they_were_queued() {
        let queue = ProcessingQueue::new(false);
        queue.enqueue(2);
        queue.enqueue(1);
        queue.enqueue(2);
        assert_eq!(next_within(&queue, NOW), Some(2));
        assert_eq!(next_within(&queue, NOW), Some(1));
        assert_eq!(next_within(&queue, A_MOMENT), None, "queued once");
    }

    #[test]
    fn a_meeting_stopped_as_meetings_turn_off_is_not_processed() {
        // Turning meetings off stops the recording, which queues the meeting,
        // then pauses the queue before the worker picks the meeting up.
        let queue = ProcessingQueue::new(false);
        queue.enqueue(1);
        queue.pause();
        assert_eq!(next_within(&queue, NOW), Some(1));
        assert!(!queue.begin(1, &operation(1)));
    }

    #[test]
    fn pausing_cancels_the_meeting_in_progress() {
        let queue = ProcessingQueue::new(false);
        let processing = operation(1);
        assert!(queue.begin(1, &processing));
        queue.pause();
        assert!(processing.is_cancelled());
    }

    #[test]
    fn a_held_meeting_is_processed_after_resuming() {
        let queue = ProcessingQueue::new(true);
        queue.enqueue(1);
        assert_eq!(next_within(&queue, NOW), Some(1));
        assert!(!queue.begin(1, &operation(1)));
        queue.resume();
        queue.enqueue(1);
        assert_eq!(next_within(&queue, NOW), Some(1));
        assert!(queue.begin(1, &operation(2)));
    }

    #[test]
    fn the_queue_waits_longer_after_each_rate_limited_meeting_in_a_row() {
        let mut hold = RateLimitHold::default();
        let mut minutes = || hold.rate_limited().as_secs() / 60;
        assert_eq!(minutes(), 30);
        assert_eq!(minutes(), 60);
        assert_eq!(minutes(), 120);
        assert_eq!(minutes(), 240);
        assert_eq!(minutes(), 240, "capped");
        // A meeting that got through: the cap has freed up.
        hold.reset();
        assert_eq!(hold.rate_limited().as_secs() / 60, 30);
    }

    #[test]
    fn the_rate_limit_hold_keeps_meetings_queued_on_their_own_waiting() {
        let queue = ProcessingQueue::new(false);
        queue.hold(AN_HOUR);
        queue.enqueue(1);
        assert_eq!(next_within(&queue, A_MOMENT), None);
    }

    #[test]
    fn a_hold_that_ran_out_while_the_mac_slept_lets_the_queue_go_on_waking() {
        let queue = ProcessingQueue::new(false);
        queue.hold(AN_HOUR);
        queue.enqueue(1);
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                let start = tokio::time::Instant::now();
                let next = queue.next();
                tokio::pin!(next);
                assert!(tokio::time::timeout(NOW, &mut next).await.is_err());
                // Asleep, the wall clock passes the end of the hold but the
                // uptime clock, which `Instant` reads on macOS, stands still.
                queue.state().held_until = Some(SystemTime::now() - A_MOMENT);
                assert_eq!(next.await, 1);
                // Within a minute of waking, not when the uptime gets there.
                assert!(start.elapsed() <= Duration::from_secs(60) + NOW);
            });
    }

    #[test]
    fn a_meeting_queued_on_its_own_goes_once_the_hold_is_over() {
        let queue = ProcessingQueue::new(false);
        queue.hold(A_MOMENT);
        queue.enqueue(1);
        assert_eq!(next_within(&queue, NOW), Some(1));
    }

    #[test]
    fn a_meeting_the_user_asked_for_during_the_hold_goes_through_it() {
        let queue = Arc::new(ProcessingQueue::new(false));
        queue.hold(AN_HOUR);
        let asking = Arc::clone(&queue);
        let ask = std::thread::spawn(move || {
            std::thread::sleep(A_MOMENT);
            asking.enqueue_asked(1);
        });
        assert_eq!(next_within(&queue, NOW), Some(1));
        ask.join().unwrap();
    }

    #[test]
    fn retry_during_the_hold_runs_only_that_meeting_ahead_of_the_waiting_ones() {
        // A was recorded during the hold; then the user retries R.
        let (a, r) = (1, 2);
        let queue = ProcessingQueue::new(false);
        queue.hold(AN_HOUR);
        queue.enqueue(a);
        queue.enqueue_asked(r);
        assert_eq!(next_within(&queue, NOW), Some(r));
        assert_eq!(next_within(&queue, A_MOMENT), None, "A still waits");
        // R got through: Codex freed up.
        queue.codex_said(CodexVerdict::Answered);
        assert_eq!(next_within(&queue, NOW), Some(a));
    }

    fn held_minutes(queue: &ProcessingQueue) -> Option<u64> {
        let until = queue.state().held_until?;
        let left = until.duration_since(SystemTime::now()).unwrap_or_default();
        Some((left.as_secs() + 30) / 60)
    }

    #[test]
    fn a_pass_codex_never_answered_keeps_the_hold() {
        // R hit the limit and A waits; then a summary retry of M goes
        // through the hold, and Codex transcribes nothing for it.
        let (a, m) = (1, 3);
        let queue = ProcessingQueue::new(false);
        assert_eq!(
            queue.codex_said(CodexVerdict::RateLimited),
            Some(Duration::from_secs(30 * 60))
        );
        queue.enqueue(a);
        queue.enqueue_asked(m);
        assert_eq!(next_within(&queue, NOW), Some(m));
        assert_eq!(queue.codex_said(CodexVerdict::Unknown), None);
        assert_eq!(next_within(&queue, A_MOMENT), None, "A still waits");
        assert_eq!(held_minutes(&queue), Some(30));
        // The next refusal is still one more in a row.
        queue.codex_said(CodexVerdict::RateLimited);
        assert_eq!(held_minutes(&queue), Some(60));
    }

    #[test]
    fn codex_answering_lifts_the_hold_and_restarts_the_wait() {
        let queue = ProcessingQueue::new(false);
        queue.codex_said(CodexVerdict::RateLimited);
        queue.codex_said(CodexVerdict::RateLimited);
        queue.enqueue(1);
        queue.codex_said(CodexVerdict::Answered);
        assert_eq!(held_minutes(&queue), None);
        assert_eq!(next_within(&queue, NOW), Some(1));
        queue.codex_said(CodexVerdict::RateLimited);
        assert_eq!(held_minutes(&queue), Some(30));
    }

    #[test]
    fn a_meeting_kept_waiting_by_the_hold_says_for_how_long_until_asked_for() {
        let queue = ProcessingQueue::new(false);
        queue.enqueue(1);
        assert_eq!(queue.held_for(1), None);
        queue.hold(AN_HOUR);
        let left = queue.held_for(1).unwrap();
        assert!(left > AN_HOUR - Duration::from_secs(60) && left <= AN_HOUR);
        assert_eq!(queue.held_for(2), None, "not queued");
        queue.enqueue_asked(1);
        assert_eq!(queue.held_for(1), None, "goes through the hold");
    }

    #[test]
    fn retrying_a_waiting_meeting_lets_it_through_the_hold() {
        let queue = ProcessingQueue::new(false);
        queue.hold(AN_HOUR);
        queue.enqueue(1);
        queue.enqueue(2);
        queue.enqueue_asked(2);
        assert_eq!(next_within(&queue, NOW), Some(2));
        assert_eq!(next_within(&queue, A_MOMENT), None);
    }

    #[test]
    fn a_meeting_asked_for_once_is_held_when_queued_again_on_its_own() {
        let queue = ProcessingQueue::new(false);
        queue.enqueue_asked(1);
        queue.hold(AN_HOUR);
        assert_eq!(next_within(&queue, NOW), Some(1));
        queue.enqueue(1);
        assert_eq!(next_within(&queue, A_MOMENT), None);
    }

    /// A connection check that answers what `online` says.
    #[derive(Clone, Default)]
    struct FakeConnection {
        online: Arc<std::sync::atomic::AtomicBool>,
        checks: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Reachability for FakeConnection {
        fn online(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
            use std::sync::atomic::Ordering;
            self.checks.fetch_add(1, Ordering::SeqCst);
            let online = self.online.load(Ordering::SeqCst);
            Box::pin(async move { online })
        }
    }

    impl FakeConnection {
        fn set_online(&self, online: bool) {
            self.online
                .store(online, std::sync::atomic::Ordering::SeqCst);
        }

        fn checks(&self) -> usize {
            self.checks.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn queue_with(connection: &FakeConnection) -> ProcessingQueue {
        let mut queue = ProcessingQueue::new(false);
        queue.reachability = Box::new(connection.clone());
        queue
    }

    /// The offline hold's wait is over, as if the run ended long ago.
    fn connection_due(queue: &ProcessingQueue) {
        queue
            .state()
            .offline
            .checked(false, SystemTime::now() - AN_HOUR);
    }

    #[test]
    fn while_codex_is_out_of_reach_only_meetings_asked_for_go_through() {
        let connection = FakeConnection::default();
        let queue = queue_with(&connection);
        assert_eq!(
            queue.codex_said(CodexVerdict::Unreachable),
            Some(Duration::from_secs(60))
        );
        queue.enqueue(1);
        queue.enqueue_asked(2);
        assert!(queue.offline_for(1));
        assert!(!queue.offline_for(2), "goes through the hold");
        assert_eq!(next_within(&queue, NOW), Some(2));
        assert_eq!(next_within(&queue, A_MOMENT), None, "1 still waits");
        // The wait is not over: the connection is not checked yet.
        assert_eq!(connection.checks(), 0);
    }

    #[test]
    fn the_queue_goes_on_once_a_check_finds_the_connection_back() {
        let connection = FakeConnection::default();
        let queue = queue_with(&connection);
        queue.codex_said(CodexVerdict::Unreachable);
        queue.enqueue(1);
        connection_due(&queue);
        assert_eq!(next_within(&queue, A_MOMENT), None, "still offline");
        assert_eq!(connection.checks(), 1);
        assert!(queue.offline_for(1));

        connection.set_online(true);
        connection_due(&queue);
        assert_eq!(next_within(&queue, NOW), Some(1));
        assert_eq!(connection.checks(), 2);
        assert!(!queue.state().offline.holds());
    }

    #[test]
    fn each_unreachable_meeting_in_a_row_doubles_the_wait_until_codex_answers() {
        let queue = queue_with(&FakeConnection::default());
        let minutes = || {
            queue
                .codex_said(CodexVerdict::Unreachable)
                .map(|wait| wait.as_secs() / 60)
        };
        let waits: Vec<_> = (0..6).map(|_| minutes()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 15, 15].map(Some));
        assert_eq!(queue.codex_said(CodexVerdict::Answered), None);
        assert!(!queue.state().offline.holds());
        assert_eq!(
            queue.codex_said(CodexVerdict::Unreachable),
            Some(Duration::from_secs(60))
        );
        // A rate limit is an answer too: the connection works.
        queue.codex_said(CodexVerdict::RateLimited);
        assert!(!queue.state().offline.holds());
    }

    #[test]
    fn offline_with_the_local_model_ready_new_meetings_still_go_through() {
        let mut queue = queue_with(&FakeConnection::default());
        queue.local_ready = Box::new(|| true);
        queue.codex_said(CodexVerdict::Unreachable);
        queue.enqueue(1);
        assert!(!queue.offline_for(1));
        assert_eq!(next_within(&queue, NOW), Some(1));

        // This Mac transcribed meeting 2 already: only Codex can finish it.
        queue.requeue(2, true);
        assert!(queue.offline_for(2));
        assert_eq!(next_within(&queue, A_MOMENT), None);
        // Meeting 3 was held before the model was ready.
        queue.requeue(3, false);
        assert_eq!(next_within(&queue, NOW), Some(3));
    }

    #[test]
    fn a_meeting_the_local_model_takes_past_the_hold_leaves_the_next_check_as_it_was() {
        let mut queue = queue_with(&FakeConnection::default());
        queue.local_ready = Box::new(|| true);
        queue.codex_said(CodexVerdict::Unreachable);
        let check = queue.state().offline.retry_at();
        queue.enqueue(1);
        assert_eq!(next_within(&queue, NOW), Some(1));
        let wait = queue.codex_said(CodexVerdict::Unreachable);
        assert!(
            wait.is_some_and(|wait| wait <= Duration::from_secs(60)),
            "{wait:?}"
        );
        assert_eq!(queue.state().offline.retry_at(), check);
        // A run the hold did not let through still counts.
        assert_eq!(
            queue.codex_said(CodexVerdict::Unreachable),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn offline_a_meeting_left_with_only_its_summary_waits_for_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
        let untranscribed = store
            .insert_recording(1_000, store::MeetingTracks::Mic)
            .unwrap();
        store
            .finish_recording(untranscribed, 61_000, 60_000)
            .unwrap();
        let transcribed = store
            .insert_recording(100_000, store::MeetingTracks::Mic)
            .unwrap();
        store
            .finish_recording(transcribed, 160_000, 60_000)
            .unwrap();
        store.begin_processing(transcribed).unwrap();
        store.finish_processing(transcribed, None).unwrap();

        let mut queue = queue_with(&FakeConnection::default());
        queue.local_ready = Box::new(|| true);
        queue.codex_said(CodexVerdict::Unreachable);
        queue_waiting(&queue, &store);
        assert!(!queue.offline_for(untranscribed), "this Mac transcribes it");
        assert!(
            queue.offline_for(transcribed),
            "only Codex can summarise it"
        );
    }

    #[test]
    fn a_rate_limit_holds_meetings_even_with_the_local_model_ready() {
        let mut queue = queue_with(&FakeConnection::default());
        queue.local_ready = Box::new(|| true);
        queue.codex_said(CodexVerdict::RateLimited);
        queue.enqueue(1);
        assert_eq!(next_within(&queue, A_MOMENT), None);
    }

    #[test]
    fn the_local_model_becoming_ready_wakes_the_meetings_waiting_offline() {
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut queue = queue_with(&FakeConnection::default());
        let flag = Arc::clone(&ready);
        queue.local_ready = Box::new(move || flag.load(std::sync::atomic::Ordering::SeqCst));
        let queue = Arc::new(queue);
        queue.codex_said(CodexVerdict::Unreachable);
        queue.enqueue(1);
        let waking = Arc::clone(&queue);
        let model = std::thread::spawn(move || {
            std::thread::sleep(A_MOMENT);
            ready.store(true, std::sync::atomic::Ordering::SeqCst);
            waking.local_model_ready();
        });
        // Well before the next connection check, a minute away.
        assert_eq!(next_within(&queue, NOW), Some(1));
        model.join().unwrap();
    }

    #[test]
    fn cancelling_one_meeting_leaves_another_running() {
        let queue = ProcessingQueue::new(false);
        let processing = operation(1);
        queue.enqueue(1);
        assert!(queue.begin(1, &processing));
        queue.cancel(2);
        assert!(!processing.is_cancelled());
        queue.cancel(1);
        assert!(processing.is_cancelled());
    }
}
