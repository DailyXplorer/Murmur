//! The on-device speech models, shared by dictation and meetings: one
//! download per Mac, every file pinned and checked by SHA-256 (`manifest`).
//! On Apple silicon they live in `app_data/meetings/models`, laid out as
//! FluidAudio expects, and load once after the download so Core ML compiles
//! them for this Mac. A marker holding the manifest's id is written last,
//! so "ready" survives restarts and a half-finished download never counts.
//! Models an earlier version downloaded are checked file by file and only
//! the differing files are fetched again. An optional source (the CTC model
//! that boosts custom words) is fetched once the others are ready, with a
//! marker of its own, and never changes the state. Models ready with the
//! previous speech model (`manifest::PREVIOUS`) stay ready and in use while
//! the current one downloads beside them; once it is ready, the previous
//! one's folder is deleted.

mod app;
pub mod download;
pub mod manifest;
mod pinned;
pub mod verify;

pub use crate::meetings::helper::AnalysisSupport;
pub use app::{init, switch_if_asked};

use serde::Serialize;
use specta::Type;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use crate::{OperationId, ProcessingOperation};
use download::{DownloadFailure, Fetch, Stopped};
use manifest::{Asr, ModelFile, Source};

const READY_MARKER: &str = ".murmur-ready";
/// Holds the optional sources' id once their files are all in place.
const OPTIONAL_MARKER: &str = ".murmur-optional";
const FLUID_REVISION_MARKER: &str = ".fluidaudio-revision";
/// Progress is sent at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LocalModelState {
    /// This Mac cannot run the models.
    Unsupported {
        support: AnalysisSupport,
    },
    Missing,
    Downloading {
        done_bytes: u64,
        total_bytes: u64,
    },
    /// Checking models an earlier version downloaded, or loading new ones
    /// once so they are compiled for this Mac.
    Preparing,
    Ready,
    Failed {
        reason: DownloadFailure,
    },
}

#[derive(Serialize, Debug, Clone, PartialEq, Type, tauri_specta::Event)]
pub struct LocalModelStatus {
    pub state: LocalModelState,
    /// What the download takes on this Mac.
    pub size_bytes: u64,
    /// What a download that stopped, missing or failed, left on this Mac.
    /// Deleting the models frees it.
    pub kept_bytes: u64,
}

/// Dictation moved to this Mac, as the user asked, once the model was ready.
#[derive(Serialize, Debug, Clone, Type, tauri_specta::Event)]
pub struct SwitchedToLocal {}

pub enum Prepared {
    Done,
    Cancelled,
    Failed,
}

pub type PrepareFuture = Pin<Box<dyn Future<Output = Prepared> + Send>>;

/// What the models need from the app. Tests pass a fake.
pub trait Host: Send + Sync + 'static {
    fn notify(&self, status: LocalModelStatus);
    /// Saves whether a download should resume at the next launch.
    fn remember_wanted(&self, wanted: bool);
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
    /// Loads the downloaded models once, offline.
    fn prepare(&self, dir: PathBuf, operation: ProcessingOperation) -> PrepareFuture;
}

/// What the models folder holds, by its ready marker.
#[derive(Debug, PartialEq, Eq)]
enum OnDisk {
    Current,
    /// Ready with the previous speech model, which runs until the current
    /// one is ready beside it.
    Previous,
    /// Models from before this manifest, never checked against it.
    Earlier,
    Absent,
}

pub struct LocalModels {
    dir: PathBuf,
    sources: &'static [Source],
    support: AnalysisSupport,
    base_url: String,
    host: Arc<dyn Host>,
    state: watch::Sender<LocalModelState>,
    download: Mutex<Option<ProcessingOperation>>,
    /// The optional sources' fetch, which ready models run on their own.
    optional: Mutex<Option<ProcessingOperation>>,
    /// The download slot's operation is `start_upgrade`'s, which a delete
    /// cancels instead of being refused.
    upgrading: AtomicBool,
}

impl LocalModels {
    pub fn new(
        dir: PathBuf,
        sources: &'static [Source],
        support: AnalysisSupport,
        base_url: String,
        host: Arc<dyn Host>,
    ) -> Arc<Self> {
        let models = Self {
            dir,
            sources,
            support,
            base_url,
            host,
            state: watch::channel(LocalModelState::Missing).0,
            download: Mutex::new(None),
            optional: Mutex::new(None),
            upgrading: AtomicBool::new(false),
        };
        let state = if support != AnalysisSupport::Available {
            LocalModelState::Unsupported { support }
        } else {
            match models.on_disk() {
                OnDisk::Current | OnDisk::Previous => LocalModelState::Ready,
                OnDisk::Earlier => LocalModelState::Preparing,
                OnDisk::Absent => LocalModelState::Missing,
            }
        };
        models.state.send_replace(state);
        Arc::new(models)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn status(&self) -> LocalModelStatus {
        let state = self.state.borrow().clone();
        let kept_bytes = match state {
            LocalModelState::Missing | LocalModelState::Failed { .. } => dir_bytes(&self.dir),
            _ => 0,
        };
        LocalModelStatus {
            state,
            size_bytes: manifest::total_bytes(self.sources),
            kept_bytes,
        }
    }

    pub fn is_ready(&self) -> bool {
        *self.state.borrow() == LocalModelState::Ready
    }

    pub fn subscribe(&self) -> watch::Receiver<LocalModelState> {
        self.state.subscribe()
    }

    /// The sources dictation waits for.
    fn required(&self) -> impl Iterator<Item = &'static Source> + Send + 'static {
        self.sources.iter().filter(|source| !source.optional)
    }

    fn set(&self, state: LocalModelState) {
        self.state.send_replace(state);
        self.host.notify(self.status());
    }

    fn download_slot(&self) -> std::sync::MutexGuard<'_, Option<ProcessingOperation>> {
        self.download
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn optional_slot(&self) -> std::sync::MutexGuard<'_, Option<ProcessingOperation>> {
        self.optional
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn on_disk(&self) -> OnDisk {
        match std::fs::read_to_string(self.dir.join(READY_MARKER)) {
            Ok(id) if id == manifest::id(self.required()) => OnDisk::Current,
            Ok(id) if id == manifest::PREVIOUS.ready_id => OnDisk::Previous,
            Ok(_) => OnDisk::Earlier,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => OnDisk::Absent,
            Err(_) => OnDisk::Earlier,
        }
    }

    /// At launch: checks models from an earlier version, and resumes a
    /// download the user asked for before the app quit. Ready models of the
    /// previous speech model fetch the current one; others get the optional
    /// files they lack.
    pub fn resume_on_launch(self: &Arc<Self>, wanted: bool) {
        let state = self.state.borrow().clone();
        match state {
            LocalModelState::Preparing => self.start_download(),
            LocalModelState::Missing if wanted => self.start_download(),
            LocalModelState::Ready if self.on_disk() == OnDisk::Previous => self.start_upgrade(),
            LocalModelState::Ready => {
                // A quit between the new ready marker and the delete left it.
                self.remove_previous();
                self.start_optional();
            }
            _ => {}
        }
    }

    /// Fetches the current speech model beside the previous one, which
    /// stays ready and in use meanwhile: the state does not change until
    /// the switch. A failure is logged, and the next launch tries again.
    fn start_upgrade(self: &Arc<Self>) {
        let operation = {
            let mut slot = self.download_slot();
            if slot.is_some() {
                return;
            }
            let operation = ProcessingOperation::new(OperationId(0));
            *slot = Some(operation.clone());
            self.upgrading.store(true, Ordering::SeqCst);
            operation
        };
        let models = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = models.fetch(&operation, false, &|_| {}).await;
            let mut slot = models.download_slot();
            *slot = None;
            models.upgrading.store(false, Ordering::SeqCst);
            match result {
                Ok(()) => {
                    log::info!("Local models switched to the current speech model");
                    models.set(LocalModelState::Ready);
                    drop(slot);
                    models.start_optional();
                }
                Err(Stopped::Cancelled) => {}
                Err(Stopped::Failed(reason)) => {
                    log::warn!("The current speech model was not fetched: {reason:?}")
                }
            }
        });
    }

    /// Deletes the previous speech model's folder, once the current one is
    /// ready. A model it loaded keeps running from the files it opened.
    fn remove_previous(&self) {
        match std::fs::remove_dir_all(self.dir.join(manifest::PREVIOUS.folder)) {
            Ok(()) => log::info!("Deleted the previous speech model"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("Cannot delete the previous speech model: {error}"),
        }
    }

    /// The state once no download or check is running.
    pub async fn settled(&self) -> LocalModelState {
        let mut state = self.state.subscribe();
        loop {
            let current = state.borrow_and_update().clone();
            let busy = matches!(
                current,
                LocalModelState::Downloading { .. } | LocalModelState::Preparing
            );
            if !busy || state.changed().await.is_err() {
                return current;
            }
        }
    }

    /// Waits for a fetch in the background, which leaves the state as it is.
    #[cfg(test)]
    pub(crate) async fn fetched(&self) {
        for _ in 0..60_000 {
            if self.download_slot().is_none() && self.optional_slot().is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the models are still fetching");
    }

    /// Waits until no download or check is running, then tells whether the
    /// models are ready. Returns `false` at once if `operation` is cancelled.
    pub async fn wait_ready(&self, operation: &ProcessingOperation) -> bool {
        tokio::select! {
            state = self.settled() => state == LocalModelState::Ready,
            () = operation.cancelled() => false,
        }
    }

    /// The models went missing under the helper, which looked for the
    /// speech model `looked_for`: download them again. Not when that model
    /// was replaced since, as the current one is not known to be missing.
    pub fn invalidate(&self, looked_for: Asr) {
        if asr_in_use(&self.dir) != looked_for {
            return;
        }
        let _ = std::fs::remove_file(self.dir.join(READY_MARKER));
        if self.is_ready() {
            self.set(LocalModelState::Missing);
        }
    }

    /// Next to the models folder, so deleting the models keeps it. Written
    /// when the user deletes the models or cancels their download; a meeting
    /// start then leaves them alone until the user downloads them again.
    fn declined_marker(&self) -> PathBuf {
        self.dir.with_extension("declined")
    }

    pub fn declined(&self) -> bool {
        self.declined_marker().exists()
    }

    /// The user asked for the models.
    pub fn download(self: &Arc<Self>) {
        match std::fs::remove_file(self.declined_marker()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::error!("Cannot clear the declined models marker: {error}"),
        }
        self.host.remember_wanted(true);
        self.start_download();
    }

    /// The user cancelled the download. Partial files stay, so the next
    /// download resumes them.
    pub fn cancel(&self) {
        self.decline();
        if let Some(operation) = self.download_slot().as_ref() {
            operation.cancel();
        }
    }

    fn decline(&self) {
        self.host.remember_wanted(false);
        if let Err(error) = std::fs::write(self.declined_marker(), b"") {
            log::error!("Cannot remember that the models were declined: {error}");
        }
    }

    /// Starts downloading in the background, unless the models are there,
    /// already coming, or cannot run on this Mac.
    pub fn start_download(self: &Arc<Self>) {
        self.start_download_unless(false);
    }

    /// `start_download`, unless the user deleted the models or cancelled
    /// their download, as a meeting start does. Checked under the slot
    /// `delete` holds, so a delete under way wins.
    pub fn download_unless_declined(self: &Arc<Self>) {
        self.start_download_unless(true);
    }

    fn start_download_unless(self: &Arc<Self>, declined: bool) {
        if self.support != AnalysisSupport::Available {
            return;
        }
        let operation = {
            let mut slot = self.download_slot();
            if slot.is_some() || self.is_ready() || (declined && self.declined()) {
                return;
            }
            let operation = ProcessingOperation::new(OperationId(0));
            *slot = Some(operation.clone());
            operation
        };
        let earlier = self.on_disk() == OnDisk::Earlier;
        self.set(if earlier {
            LocalModelState::Preparing
        } else {
            LocalModelState::Downloading {
                done_bytes: 0,
                total_bytes: manifest::total_bytes(self.required()),
            }
        });
        let models = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let result = models
                .fetch(&operation, earlier, &|state| models.set(state))
                .await;
            let mut slot = models.download_slot();
            *slot = None;
            match result {
                Ok(()) => {
                    log::info!("Local models ready");
                    models.set(LocalModelState::Ready);
                    models.start_optional();
                }
                Err(Stopped::Cancelled) => models.set(LocalModelState::Missing),
                Err(Stopped::Failed(reason)) => {
                    log::warn!("Local models download failed: {reason:?}");
                    models.set(LocalModelState::Failed { reason });
                }
            }
        });
    }

    /// Fetches the optional files ready models lack, in the background.
    fn start_optional(self: &Arc<Self>) {
        let operation = {
            let mut slot = self.optional_slot();
            if slot.is_some() {
                return;
            }
            let operation = ProcessingOperation::new(OperationId(0));
            *slot = Some(operation.clone());
            operation
        };
        let models = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            models.fetch_optional(&operation).await;
            *models.optional_slot() = None;
        });
    }

    /// Cancels the optional fetch and waits for it to stop, so it writes
    /// nothing more into the folder.
    fn stop_optional(&self) -> Result<(), String> {
        if let Some(operation) = self.optional_slot().as_ref() {
            operation.cancel();
        }
        wait_until_stopped(|| self.optional_slot().is_none())
    }

    /// Cancels the background fetch of the current speech model and waits
    /// for it to stop: the user's delete wins over it. Any other download
    /// is left alone, and the delete refuses it.
    fn stop_upgrade(&self) -> Result<(), String> {
        match self.download_slot().as_ref() {
            Some(operation) if self.upgrading.load(Ordering::SeqCst) => {
                operation.cancel();
            }
            _ => return Ok(()),
        }
        wait_until_stopped(|| self.download_slot().is_none())
    }

    /// The state stays as it is: a failure is logged, and the next launch
    /// tries again.
    async fn fetch_optional(&self, operation: &ProcessingOperation) {
        let optional: Vec<&'static Source> = self.sources.iter().filter(|s| s.optional).collect();
        if optional.is_empty() {
            return;
        }
        let id = manifest::id(optional.iter().copied());
        let marker = self.dir.join(OPTIONAL_MARKER);
        if std::fs::read_to_string(&marker).is_ok_and(|held| held == id) {
            return;
        }
        let dir = self.dir.clone();
        let fetched = async {
            let missing =
                tauri::async_runtime::spawn_blocking(move || missing_files(&dir, optional))
                    .await
                    .map_err(|_| Stopped::Failed(DownloadFailure::Disk))??;
            let client = download::client();
            let fetch = Fetch {
                client: &client,
                operation,
                stall: download::STALL_TIMEOUT,
            };
            for (source, file) in &missing {
                let url = source.url(&self.base_url, file);
                fetch
                    .file(&url, &self.path_of(source, file), file, &mut |_| {})
                    .await?;
            }
            // Core ML compiles a new model on its first load, about 12 s for
            // the CTC model: done here, the first dictation does not wait.
            match self.host.prepare(self.dir.clone(), operation.clone()).await {
                Prepared::Done => {}
                Prepared::Cancelled => return Err(Stopped::Cancelled),
                Prepared::Failed => return Err(Stopped::Failed(DownloadFailure::PrepareFailed)),
            }
            std::fs::write(&marker, &id)?;
            Ok::<_, Stopped>(())
        };
        if let Err(Stopped::Failed(reason)) = fetched.await {
            log::warn!("Optional local model files not fetched: {reason:?}");
        }
    }

    /// Fetches what the required sources lack, then prepares them. `show`
    /// gets the states it goes through.
    async fn fetch(
        &self,
        operation: &ProcessingOperation,
        earlier: bool,
        show: &(dyn Fn(LocalModelState) + Sync),
    ) -> Result<(), Stopped> {
        std::fs::create_dir_all(&self.dir)?;
        let (dir, sources) = (self.dir.clone(), self.required());
        let missing = tauri::async_runtime::spawn_blocking(move || missing_files(&dir, sources))
            .await
            .map_err(|_| Stopped::Failed(DownloadFailure::Disk))??;
        // The check removed the wrong files: the folder is no longer
        // complete, so whether to download them is the user's call.
        if earlier && !missing.is_empty() {
            match std::fs::remove_file(self.dir.join(READY_MARKER)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if operation.is_cancelled() {
            return Err(Stopped::Cancelled);
        }
        let total = manifest::total_bytes(self.required());
        let mut done = total - missing.iter().map(|(_, file)| file.size).sum::<u64>();
        let remaining: u64 = missing
            .iter()
            .map(|(source, file)| file.size - self.partial_bytes(source, file))
            .sum();
        if remaining > 0 {
            let needed = remaining + remaining / 10;
            if let Some(free) = self.host.free_bytes(&self.dir) {
                if free < needed {
                    return Err(Stopped::Failed(DownloadFailure::NotEnoughSpace {
                        needed_bytes: needed,
                        free_bytes: free,
                    }));
                }
            }
        }
        if !missing.is_empty() {
            log::info!("Downloading {} local model file(s)", missing.len());
            show(LocalModelState::Downloading {
                done_bytes: done,
                total_bytes: total,
            });
        }
        let client = download::client();
        let fetch = Fetch {
            client: &client,
            operation,
            stall: download::STALL_TIMEOUT,
        };
        let mut last_sent = Instant::now();
        for (source, file) in &missing {
            let before = done;
            fetch
                .file(
                    &source.url(&self.base_url, file),
                    &self.path_of(source, file),
                    file,
                    &mut |bytes| {
                        if last_sent.elapsed() >= PROGRESS_EVERY {
                            last_sent = Instant::now();
                            show(LocalModelState::Downloading {
                                done_bytes: before + bytes,
                                total_bytes: total,
                            });
                        }
                    },
                )
                .await?;
            done += file.size;
        }
        for source in self.required().filter(|s| s.fluid_revision_marker) {
            std::fs::write(
                self.dir.join(source.folder).join(FLUID_REVISION_MARKER),
                format!("{}\n", source.commit),
            )?;
        }
        // Models an earlier version checked and compiled need no new load.
        if !(earlier && missing.is_empty()) {
            show(LocalModelState::Preparing);
            match self.host.prepare(self.dir.clone(), operation.clone()).await {
                Prepared::Done => {}
                Prepared::Cancelled => return Err(Stopped::Cancelled),
                Prepared::Failed => return Err(Stopped::Failed(DownloadFailure::PrepareFailed)),
            }
        }
        std::fs::write(self.dir.join(READY_MARKER), manifest::id(self.required()))?;
        self.remove_previous();
        Ok(())
    }

    fn path_of(&self, source: &Source, file: &ModelFile) -> PathBuf {
        self.dir.join(source.folder).join(file.path)
    }

    fn partial_bytes(&self, source: &Source, file: &ModelFile) -> u64 {
        std::fs::metadata(download::part_path(&self.path_of(source, file)))
            .map_or(0, |metadata| metadata.len().min(file.size))
    }

    /// Deletes the models. Refused while they download; the current speech
    /// model fetched beside the previous one, and the optional files ready
    /// models are still fetching, are given up. Blocks until those fetches
    /// stop. No download starts until it returns.
    pub fn delete(&self) -> Result<(), String> {
        self.stop_upgrade()?;
        let slot = self.download_slot();
        if slot.is_some() {
            return Err("The models are downloading".to_string());
        }
        self.stop_optional()?;
        self.decline();
        // The markers go first: a delete stopped halfway, as Windows does on
        // a file another program holds open, leaves models that are not ready
        // and a custom-word model the next download fetches again.
        for marker in [OPTIONAL_MARKER, READY_MARKER] {
            match std::fs::remove_file(self.dir.join(marker)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Cannot delete the models: {error}")),
            }
        }
        let removed = match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Cannot delete the models: {error}")),
        };
        if self.support == AnalysisSupport::Available {
            self.set(LocalModelState::Missing);
        }
        drop(slot);
        removed
    }
}

/// Polls `stopped` until it holds, for at most 10 s.
fn wait_until_stopped(stopped: impl Fn() -> bool) -> Result<(), String> {
    const STOP_WITHIN: Duration = Duration::from_secs(10);
    let started = Instant::now();
    while !stopped() {
        if started.elapsed() > STOP_WITHIN {
            return Err("The models are downloading".to_string());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Whether this build's optional sources are in place in the models folder
/// `dir`: their marker is written once they are fetched and prepared. An
/// earlier pin's marker stays while its files are replaced.
pub fn optional_ready(dir: &Path) -> bool {
    let optional = manifest::this_mac().iter().filter(|source| source.optional);
    std::fs::read_to_string(dir.join(OPTIONAL_MARKER)).is_ok_and(|id| id == manifest::id(optional))
}

/// The speech model the models folder `dir` runs: the previous one until
/// the current one is ready beside it.
pub fn asr_in_use(dir: &Path) -> Asr {
    match std::fs::read_to_string(dir.join(READY_MARKER)) {
        Ok(id) if id == manifest::PREVIOUS.ready_id => Asr::Previous,
        _ => Asr::Current,
    }
}

/// The size of every file under `dir`, 0 when it is absent.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_dir() => dir_bytes(&entry.path()),
            Ok(metadata) => metadata.len(),
            Err(_) => 0,
        })
        .sum()
}

/// The files not yet on disk with the right content. A wrong file is
/// removed; partial downloads stay to be resumed.
fn missing_files(
    dir: &Path,
    sources: impl IntoIterator<Item = &'static Source>,
) -> std::io::Result<Vec<(&'static Source, &'static ModelFile)>> {
    let mut missing = Vec::new();
    for source in sources {
        for file in source.files {
            let path = dir.join(source.folder).join(file.path);
            if verify::matches(&path, file)? {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => log::info!("{} differs from the manifest", file.path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            missing.push((source, file));
        }
    }
    Ok(missing)
}

#[cfg(test)]
pub(crate) mod tests;
