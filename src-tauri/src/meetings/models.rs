//! The models that tell speakers apart (FluidAudio: Parakeet v3 for word
//! times, pyannote Community-1 for diarization) are the shared local models
//! (`local_models`), one download with dictation. Meetings add one rule: a
//! meeting start fetches them while it records, unless the user deleted
//! them or cancelled their download.

use std::sync::Arc;

pub use crate::local_models::LocalModels as Models;

impl Models {
    pub fn download_unless_declined(self: &Arc<Self>) {
        if !self.declined() {
            self.start_download();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::local_models::manifest::{ModelFile, Source};
    use crate::local_models::{Host, LocalModelState, LocalModels, PrepareFuture, Prepared};
    use crate::meetings::helper::AnalysisSupport;
    use crate::ProcessingOperation;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    struct NoHost;

    impl Host for NoHost {
        fn notify(&self, _: crate::local_models::LocalModelStatus) {}
        fn remember_wanted(&self, _: bool) {}
        fn free_bytes(&self, _: &Path) -> Option<u64> {
            None
        }
        fn prepare(&self, _: PathBuf, _: ProcessingOperation) -> PrepareFuture {
            Box::pin(async { Prepared::Done })
        }
    }

    const FILES: &[ModelFile] = &[ModelFile {
        path: "model.bin",
        size: 4,
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    }];
    const SOURCES: &[Source] = &[Source {
        repo: "test/models",
        commit: "1111111111111111111111111111111111111111",
        folder: "models",
        fluid_revision_marker: false,
        files: FILES,
    }];

    /// Nothing listens on the discard port, so a download fails at once.
    fn models(dir: &Path) -> Arc<LocalModels> {
        LocalModels::new(
            dir.to_path_buf(),
            SOURCES,
            AnalysisSupport::Available,
            "http://127.0.0.1:9".into(),
            Arc::new(NoHost),
        )
    }

    #[test]
    fn deleted_models_stay_deleted_until_asked_for_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models");
        std::fs::create_dir_all(&path).unwrap();
        models(&path).delete().unwrap();

        // A meeting starts, even after a restart.
        let after_restart = models(&path);
        after_restart.download_unless_declined();
        assert_eq!(after_restart.status().state, LocalModelState::Missing);

        after_restart.download();
        assert_ne!(after_restart.status().state, LocalModelState::Missing);
        assert!(!path.with_extension("declined").exists());
    }

    #[test]
    fn a_download_the_user_cancelled_does_not_start_with_the_next_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models");
        let missing = models(&path);
        missing.cancel();
        missing.download_unless_declined();
        assert_eq!(missing.status().state, LocalModelState::Missing);
    }
}
