//! The on-device models that tell speakers apart (FluidAudio: Parakeet v3 for
//! word times, pyannote Community-1 for diarization), downloaded on first
//! use into `app_data/meetings/models`. A marker file is written once the
//! helper has downloaded and compiled them all, so "ready" survives restarts
//! and a half-finished download never counts.

use serde::Serialize;
use specta::Type;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use super::helper::{self, AnalysisSupport, Ended};
use super::protocol::{Message, Phase};
use crate::{OperationId, ProcessingOperation};

const READY_MARKER: &str = ".murmur-ready";
/// Progress is sent at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// About 480 MB over a slow line, then the first compile.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelsState {
    /// This Mac cannot run the analysis.
    Unsupported {
        support: AnalysisSupport,
    },
    Missing,
    Downloading {
        fraction: f32,
    },
    /// Compiling the models for this Mac, once.
    Preparing,
    Ready,
    Failed,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type, tauri_specta::Event)]
pub struct MeetingModelsStatus {
    pub state: ModelsState,
    pub bytes_on_disk: u64,
}

pub struct Models {
    dir: PathBuf,
    support: AnalysisSupport,
    state: watch::Sender<ModelsState>,
    download: Mutex<Option<ProcessingOperation>>,
    notify: Box<dyn Fn(MeetingModelsStatus) + Send + Sync>,
}

/// Bytes of the files under `dir`, without following links.
fn size_of(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => size_of(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

impl Models {
    pub fn new(
        dir: PathBuf,
        support: AnalysisSupport,
        notify: impl Fn(MeetingModelsStatus) + Send + Sync + 'static,
    ) -> Arc<Self> {
        let state = if support != AnalysisSupport::Available {
            ModelsState::Unsupported { support }
        } else if dir.join(READY_MARKER).is_file() {
            ModelsState::Ready
        } else {
            ModelsState::Missing
        };
        Arc::new(Self {
            dir,
            support,
            state: watch::channel(state).0,
            download: Mutex::new(None),
            notify: Box::new(notify),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn status(&self) -> MeetingModelsStatus {
        MeetingModelsStatus {
            state: self.state.borrow().clone(),
            bytes_on_disk: size_of(&self.dir),
        }
    }

    pub fn is_ready(&self) -> bool {
        *self.state.borrow() == ModelsState::Ready
    }

    fn set(&self, state: ModelsState) {
        self.state.send_replace(state);
        (self.notify)(self.status());
    }

    fn download_slot(&self) -> std::sync::MutexGuard<'_, Option<ProcessingOperation>> {
        self.download
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits until no download is running, then tells whether the models are
    /// ready. Returns `false` at once if `operation` is cancelled.
    pub async fn wait_ready(&self, operation: &ProcessingOperation) -> bool {
        let mut state = self.state.subscribe();
        loop {
            match &*state.borrow_and_update() {
                ModelsState::Downloading { .. } | ModelsState::Preparing => {}
                ready => return *ready == ModelsState::Ready,
            }
            tokio::select! {
                changed = state.changed() => if changed.is_err() { return false },
                () = operation.cancelled() => return false,
            }
        }
    }

    /// The models went missing under the helper: download them again.
    pub fn invalidate(&self) {
        let _ = std::fs::remove_file(self.dir.join(READY_MARKER));
        if self.is_ready() {
            self.set(ModelsState::Missing);
        }
    }

    /// Starts downloading in the background, unless the models are there,
    /// already coming, or cannot run on this Mac.
    pub fn start_download(self: &Arc<Self>) {
        if self.support != AnalysisSupport::Available {
            return;
        }
        let operation = {
            let mut slot = self.download_slot();
            if slot.is_some() || self.is_ready() {
                return;
            }
            let operation = ProcessingOperation::new(OperationId(0));
            *slot = Some(operation.clone());
            operation
        };
        self.set(ModelsState::Downloading { fraction: 0.0 });
        let models = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let state = models.run_download(&operation).await;
            *models.download_slot() = None;
            models.set(state);
        });
    }

    async fn run_download(&self, operation: &ProcessingOperation) -> ModelsState {
        let Some(program) = helper::helper_path() else {
            log::error!("The meetings helper is missing from the app");
            return ModelsState::Failed;
        };
        if let Err(error) = std::fs::create_dir_all(&self.dir) {
            log::error!("Cannot create the meeting models folder: {error}");
            return ModelsState::Failed;
        }
        let args: Vec<OsString> = vec![
            "download".into(),
            "--models-dir".into(),
            self.dir.clone().into_os_string(),
        ];
        let mut last_sent = Instant::now() - PROGRESS_EVERY;
        let mut last_state = ModelsState::Downloading { fraction: 0.0 };
        let ended = helper::run(&program, &args, operation, DOWNLOAD_TIMEOUT, |message| {
            if let Message::Progress {
                phase, fraction, ..
            } = message
            {
                let state = match phase {
                    Phase::Compile => ModelsState::Preparing,
                    _ => ModelsState::Downloading {
                        fraction: fraction.clamp(0.0, 1.0) as f32,
                    },
                };
                let phase_changed =
                    std::mem::discriminant(&state) != std::mem::discriminant(&last_state);
                if phase_changed || last_sent.elapsed() >= PROGRESS_EVERY {
                    last_sent = Instant::now();
                    last_state = state.clone();
                    self.set(state);
                }
            }
        })
        .await;
        match ended {
            Ok(Ended::Done) => match std::fs::write(self.dir.join(READY_MARKER), b"") {
                Ok(()) => {
                    log::info!("Meeting models ready");
                    ModelsState::Ready
                }
                Err(error) => {
                    log::error!("Cannot mark the meeting models ready: {error}");
                    ModelsState::Failed
                }
            },
            // Partial files stay; the next download resumes them.
            Ok(Ended::Cancelled) => ModelsState::Missing,
            Ok(ended) => {
                log::warn!("Meeting models download ended: {ended:?}");
                ModelsState::Failed
            }
            Err(error) => {
                log::error!("Cannot run the meetings helper: {error}");
                ModelsState::Failed
            }
        }
    }

    pub fn cancel_download(&self) {
        if let Some(operation) = self.download_slot().as_ref() {
            operation.cancel();
        }
    }

    /// Deletes the models. Refused while they download.
    pub fn delete(&self) -> Result<(), String> {
        if self.download_slot().is_some() {
            return Err("The models are downloading".to_string());
        }
        let _ = std::fs::remove_file(self.dir.join(READY_MARKER));
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot delete the models: {error}")),
        }
        if self.support == AnalysisSupport::Available {
            self.set(ModelsState::Missing);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(dir: &Path, support: AnalysisSupport) -> Arc<Models> {
        Models::new(dir.to_path_buf(), support, |_| {})
    }

    #[test]
    fn the_marker_decides_ready_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models");
        assert_eq!(
            models(&path, AnalysisSupport::Available).status().state,
            ModelsState::Missing
        );
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("model.bin"), [0u8; 10]).unwrap();
        std::fs::write(path.join(READY_MARKER), b"").unwrap();
        let ready = models(&path, AnalysisSupport::Available);
        assert!(ready.is_ready());
        assert_eq!(ready.status().bytes_on_disk, 10);
        ready.invalidate();
        assert_eq!(ready.status().state, ModelsState::Missing);
        assert!(!path.join(READY_MARKER).exists());
    }

    #[test]
    fn unsupported_macs_never_download() {
        let dir = tempfile::tempdir().unwrap();
        let old = models(dir.path(), AnalysisSupport::OsTooOld);
        old.start_download();
        assert_eq!(
            old.status().state,
            ModelsState::Unsupported {
                support: AnalysisSupport::OsTooOld
            }
        );
    }

    #[test]
    fn delete_removes_everything_and_reports_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models");
        std::fs::create_dir_all(path.join("sub")).unwrap();
        std::fs::write(path.join("sub/model.bin"), [0u8; 4]).unwrap();
        std::fs::write(path.join(READY_MARKER), b"").unwrap();
        let ready = models(&path, AnalysisSupport::Available);
        ready.delete().unwrap();
        assert!(!path.exists());
        assert_eq!(ready.status().state, ModelsState::Missing);
        assert_eq!(ready.status().bytes_on_disk, 0);
    }

    #[test]
    fn waiting_returns_at_once_when_nothing_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let missing = models(dir.path(), AnalysisSupport::Available);
        let operation = ProcessingOperation::new(OperationId(1));
        assert!(!tauri::async_runtime::block_on(
            missing.wait_ready(&operation)
        ));
    }
}
