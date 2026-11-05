use anyhow::{anyhow, Result};
use log::error;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::{HistoryManager, RECORDING_UNAVAILABLE_ERROR};

impl HistoryManager {
    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    pub(super) fn unavailable_recording_error() -> anyhow::Error {
        anyhow!(RECORDING_UNAVAILABLE_ERROR)
    }

    pub(super) fn resolve_recording_path_in_dir(
        recordings_dir: &Path,
        file_name: &str,
    ) -> Result<PathBuf> {
        let path = Path::new(file_name);
        let mut components = path.components();
        let has_one_normal_component =
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();

        if file_name.is_empty()
            || file_name.starts_with('.')
            || file_name.contains(['/', '\\'])
            || !file_name.ends_with(".wav")
            || !has_one_normal_component
        {
            return Err(Self::unavailable_recording_error());
        }

        let root_metadata = fs::symlink_metadata(recordings_dir)
            .map_err(|_| Self::unavailable_recording_error())?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(Self::unavailable_recording_error());
        }

        let canonical_root =
            fs::canonicalize(recordings_dir).map_err(|_| Self::unavailable_recording_error())?;
        let candidate = recordings_dir.join(file_name);
        let metadata =
            fs::symlink_metadata(&candidate).map_err(|_| Self::unavailable_recording_error())?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Self::unavailable_recording_error());
        }

        let canonical_candidate =
            fs::canonicalize(candidate).map_err(|_| Self::unavailable_recording_error())?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(Self::unavailable_recording_error());
        }

        Ok(canonical_candidate)
    }

    pub fn resolve_persisted_recording_path(&self, file_name: &str) -> Result<PathBuf> {
        Self::resolve_recording_path_in_dir(&self.recordings_dir, file_name)
    }

    pub(super) fn delete_recording_file(recordings_dir: &Path, file_name: &str) -> bool {
        let Ok(file_path) = Self::resolve_recording_path_in_dir(recordings_dir, file_name) else {
            return false;
        };

        if fs::remove_file(file_path).is_err() {
            error!("Failed to delete history recording");
            return false;
        }

        true
    }
}
