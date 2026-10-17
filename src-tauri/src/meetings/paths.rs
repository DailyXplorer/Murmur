//! Each meeting keeps its files in `app_data/meetings/<id>/`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MIC_TRACK: &str = "mic.wav";
pub const SYSTEM_TRACK: &str = "system.wav";
/// Both tracks, compressed, once processing is done.
pub const AUDIO_FILE: &str = "audio.m4a";

pub fn meeting_dir(root: &Path, id: i64) -> PathBuf {
    root.join(id.to_string())
}

/// The meeting's folder, when neither it nor the meetings root is a
/// symlink, so a planted link can never make a deletion reach files outside
/// the app's data. `None` when either does not exist.
fn safe_meeting_dir(root: &Path, id: i64) -> io::Result<Option<PathBuf>> {
    let refuse = || io::Error::new(io::ErrorKind::InvalidInput, "unsafe meeting folder");
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(refuse());
    }
    let dir = meeting_dir(root, id);
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(refuse()),
        Ok(_) => Ok(Some(dir)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Deletes a meeting's folder. A folder that does not exist is already
/// deleted.
pub fn remove_meeting_dir(root: &Path, id: i64) -> io::Result<()> {
    match safe_meeting_dir(root, id)? {
        // `remove_dir_all` does not follow symlinks inside the folder.
        Some(dir) => fs::remove_dir_all(dir),
        None => Ok(()),
    }
}

/// Deletes the named files of a meeting, those that exist. A symlink is
/// removed, never its target.
pub fn remove_meeting_files(root: &Path, id: i64, names: &[&str]) -> io::Result<()> {
    let Some(dir) = safe_meeting_dir(root, id)? else {
        return Ok(());
    };
    for name in names {
        match fs::remove_file(dir.join(name)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn removes_the_meeting_folder_and_its_tracks() {
        let root = tempfile::tempdir().unwrap();
        let dir = meeting_dir(root.path(), 7);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MIC_TRACK), b"audio").unwrap();

        remove_meeting_dir(root.path(), 7).unwrap();

        assert!(!dir.exists());
        assert!(root.path().exists());
        remove_meeting_dir(root.path(), 7).unwrap();
    }

    #[test]
    fn a_symlinked_folder_or_root_is_never_followed() {
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("keep.txt");
        fs::write(&sentinel, b"keep").unwrap();

        let root = tempfile::tempdir().unwrap();
        symlink(outside.path(), meeting_dir(root.path(), 3)).unwrap();
        assert!(remove_meeting_dir(root.path(), 3).is_err());

        let parent = tempfile::tempdir().unwrap();
        let linked_root = parent.path().join("meetings");
        symlink(outside.path(), &linked_root).unwrap();
        fs::create_dir(outside.path().join("4")).unwrap();
        assert!(remove_meeting_dir(&linked_root, 4).is_err());

        assert!(sentinel.exists());
        assert!(outside.path().join("4").exists());
    }

    #[test]
    fn named_files_are_removed_and_the_rest_kept() {
        let root = tempfile::tempdir().unwrap();
        let dir = meeting_dir(root.path(), 8);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(AUDIO_FILE), b"audio").unwrap();
        fs::write(dir.join("keep.json"), b"{}").unwrap();

        remove_meeting_files(root.path(), 8, &[AUDIO_FILE, MIC_TRACK]).unwrap();

        assert!(!dir.join(AUDIO_FILE).exists());
        assert!(dir.join("keep.json").exists());
        remove_meeting_files(root.path(), 9, &[AUDIO_FILE]).unwrap();

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join(AUDIO_FILE), b"keep").unwrap();
        symlink(outside.path(), meeting_dir(root.path(), 10)).unwrap();
        assert!(remove_meeting_files(root.path(), 10, &[AUDIO_FILE]).is_err());
        assert!(outside.path().join(AUDIO_FILE).exists());
    }

    #[test]
    fn links_inside_the_folder_are_removed_without_touching_their_targets() {
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("keep.txt");
        fs::write(&sentinel, b"keep").unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = meeting_dir(root.path(), 5);
        fs::create_dir_all(&dir).unwrap();
        symlink(outside.path(), dir.join("link")).unwrap();

        remove_meeting_dir(root.path(), 5).unwrap();

        assert!(!dir.exists());
        assert!(sentinel.exists());
    }
}
