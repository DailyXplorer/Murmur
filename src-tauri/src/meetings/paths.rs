//! Each meeting keeps its files in `app_data/meetings/<id>/`.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use specta::Type;

use super::retention;

pub const MIC_TRACK: &str = "mic.wav";
pub const SYSTEM_TRACK: &str = "system.wav";
/// Both tracks, compressed, once processing is done.
pub const AUDIO_FILE: &str = "audio.m4a";
/// Every file of a meeting that holds audio, half-written ones included.
pub const AUDIO_FILES: [&str; 6] = [
    AUDIO_FILE,
    MIC_TRACK,
    SYSTEM_TRACK,
    "audio.m4a.part",
    "mic.wav.part",
    "system.wav.part",
];

/// Where `name` is written before it is complete.
pub fn partial(name: &str) -> String {
    format!("{name}.part")
}

pub fn meeting_dir(root: &Path, id: i64) -> PathBuf {
    root.join(id.to_string())
}

/// Whether `name` is a meeting's folder, or one `fresh_meeting_dir` moved
/// aside.
fn holds_recordings(name: &str) -> bool {
    let id = name.split_once(".stale-").map_or(name, |(id, _)| id);
    id.parse::<i64>().is_ok()
}

/// After `meetings.db` was found corrupt and moved to `moved_to`, moves the
/// meetings' folders next to it, to `meetings.corrupt-<time>`: no row points
/// to them any more, so no screen would list or delete them. The models
/// stay.
pub fn set_aside_recordings(root: &Path, moved_to: &Path) -> io::Result<()> {
    let suffix = moved_to
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("meetings.db"))
        .unwrap_or(".corrupt");
    let aside = root.with_file_name(format!("meetings{suffix}"));
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let is_meeting = entry.file_name().to_str().is_some_and(holds_recordings);
        if is_meeting && entry.file_type()?.is_dir() {
            fs::create_dir_all(&aside)?;
            fs::rename(entry.path(), aside.join(entry.file_name()))?;
        }
    }
    Ok(())
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

/// Gives new meeting `id` an empty folder. Files already under it belong to
/// no meeting: before ids stopped being reused, a deleted newest meeting
/// whose folder failed to delete left them under the id the next meeting
/// gets. When they cannot be deleted they
/// are moved aside to `<id>.stale-<now_ms>`, which no meeting reads, so the
/// same failure never blocks every later meeting.
pub fn fresh_meeting_dir(root: &Path, id: i64, now_ms: i64) -> io::Result<PathBuf> {
    if let Err(error) = remove_meeting_dir(root, id) {
        // Inside a linked root, even a rename would act outside the app's data.
        if fs::symlink_metadata(root)?.file_type().is_symlink() {
            return Err(error);
        }
        log::warn!("Meeting {id}: cannot delete a stale folder, moving it aside: {error}");
        fs::rename(
            meeting_dir(root, id),
            root.join(format!("{id}.stale-{now_ms}")),
        )?;
    }
    let dir = meeting_dir(root, id);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Deletes the folders `fresh_meeting_dir` moved aside: they were given up
/// on, and no screen lists them, so nothing else ever would. Returns how
/// many went.
pub fn remove_stale(root: &Path) -> usize {
    if fs::symlink_metadata(root).is_ok_and(|root| root.file_type().is_symlink()) {
        return 0;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_stale = name
            .split_once(".stale-")
            .is_some_and(|(id, _)| id.parse::<i64>().is_ok());
        if !is_stale || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        match fs::remove_dir_all(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) => log::warn!("Cannot delete the stale meeting folder {name}: {error}"),
        }
    }
    removed
}

/// Moves the meeting folders no row points to into `aside`
/// (`meetings.orphaned-<time>`). A missing row does not mean the user
/// deleted the meeting: `meetings.db` may have been recreated empty, after
/// it was lost or moved. Deleting would lose that audio, and leaving it
/// would let a new meeting that gets the same id delete it. Returns how
/// many moved.
pub fn set_aside_orphans(root: &Path, meetings: &HashSet<i64>, aside: &Path) -> io::Result<usize> {
    let mut moved = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let is_orphan = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i64>().ok())
            .is_some_and(|id| !meetings.contains(&id));
        if is_orphan && entry.file_type()?.is_dir() {
            fs::create_dir_all(aside)?;
            fs::rename(entry.path(), aside.join(entry.file_name()))?;
            moved += 1;
        }
    }
    Ok(moved)
}

/// Deletes what was set aside next to the library (in `app_data`) more than
/// 30 days before `now`. Only entries directly in `app_data` go, and never a
/// symlink: a planted link can make no deletion reach outside the app's
/// data. Returns how many went.
pub fn purge_set_asides(app_data: &Path, now: DateTime<Utc>) -> usize {
    let Ok(entries) = fs::read_dir(app_data) else {
        return 0;
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let mut removed = 0;
    for name in retention::expired_set_asides(names.iter().map(String::as_str), now) {
        let path = app_data.join(&name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let result = if metadata.file_type().is_symlink() {
            continue;
        } else if metadata.is_dir() {
            // `remove_dir_all` does not follow symlinks inside the folder.
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed += 1,
            Err(error) => log::warn!("Cannot delete {name}, set aside long ago: {error}"),
        }
    }
    removed
}

/// The disk space meetings take, in bytes.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
pub struct MeetingsStorage {
    /// The meetings' folders: their tracks, compressed or not.
    pub audio_bytes: u64,
    /// What was set aside next to the library, deleted after 30 days.
    pub set_aside_bytes: u64,
}

/// The size of a file, or of everything in a folder. Symlinks count for
/// nothing: what they point to is not the meetings'.
fn size_of(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_dir() {
        fs::read_dir(path).map_or(0, |entries| {
            entries.flatten().map(|entry| size_of(&entry.path())).sum()
        })
    } else if metadata.is_file() {
        metadata.len()
    } else {
        0
    }
}

/// Measures the meetings under `app_data`. The speech models in
/// `meetings/models` are dictation's too, and are not counted.
pub fn storage(app_data: &Path) -> MeetingsStorage {
    let root = app_data.join("meetings");
    let set_aside_bytes = fs::read_dir(app_data).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(retention::is_set_aside)
            })
            .map(|entry| size_of(&entry.path()))
            .sum()
    });
    MeetingsStorage {
        audio_bytes: size_of(&root).saturating_sub(size_of(&root.join("models"))),
        set_aside_bytes,
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
    fn stale_folders_are_removed_and_the_rest_kept() {
        let root = tempfile::tempdir().unwrap();
        for name in ["1", "3.stale-1700000000000", "models"] {
            fs::create_dir_all(root.path().join(name)).unwrap();
            fs::write(root.path().join(name).join(MIC_TRACK), b"audio").unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("4.stale-1")).unwrap();

        assert_eq!(remove_stale(root.path()), 1);

        assert!(root.path().join("1").join(MIC_TRACK).exists());
        assert!(root.path().join("models").join(MIC_TRACK).exists());
        assert!(!root.path().join("3.stale-1700000000000").exists());
        assert!(outside.path().exists());
    }

    #[test]
    fn a_library_recreated_empty_never_deletes_the_recordings() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("7")).unwrap();
        fs::write(root.path().join("7").join(AUDIO_FILE), b"audio").unwrap();

        assert_eq!(remove_stale(root.path()), 0);

        assert!(root.path().join("7").join(AUDIO_FILE).exists());
    }

    #[test]
    fn folders_no_row_points_to_are_set_aside_and_the_rest_kept() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("meetings");
        for name in ["1", "2", "models"] {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join(MIC_TRACK), b"audio").unwrap();
        }
        let aside = parent.path().join("meetings.orphaned-20261017-101500");

        assert_eq!(
            set_aside_orphans(&root, &HashSet::from([1]), &aside).unwrap(),
            1
        );

        assert!(root.join("1").join(MIC_TRACK).exists());
        assert!(root.join("models").join(MIC_TRACK).exists());
        assert!(aside.join("2").join(MIC_TRACK).exists());
        assert!(!root.join("2").exists());

        let untouched = parent.path().join("meetings.orphaned-later");
        assert_eq!(
            set_aside_orphans(&root, &HashSet::from([1]), &untouched).unwrap(),
            0
        );
        assert!(!untouched.exists());
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

    #[test]
    fn the_half_written_names_are_those_the_archive_writes() {
        for name in [AUDIO_FILE, MIC_TRACK, SYSTEM_TRACK] {
            assert!(AUDIO_FILES.contains(&partial(name).as_str()), "{name}");
        }
    }

    #[test]
    fn deleting_the_audio_also_deletes_half_written_audio() {
        let root = tempfile::tempdir().unwrap();
        let dir = meeting_dir(root.path(), 6);
        fs::create_dir_all(&dir).unwrap();
        for name in [AUDIO_FILE, MIC_TRACK, SYSTEM_TRACK] {
            fs::write(dir.join(partial(name)), b"audio").unwrap();
        }

        remove_meeting_files(root.path(), 6, &AUDIO_FILES).unwrap();

        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    fn days_ago(now: DateTime<Utc>, days: i64) -> String {
        (now - chrono::TimeDelta::days(days))
            .format("%Y%m%d-%H%M%S")
            .to_string()
    }

    #[test]
    fn only_real_folders_set_aside_over_30_days_ago_are_purged() {
        let now = Utc::now();
        let app_data = tempfile::tempdir().unwrap();
        let old = app_data
            .path()
            .join(format!("meetings.orphaned-{}", days_ago(now, 31)));
        let recent = app_data
            .path()
            .join(format!("meetings.orphaned-{}", days_ago(now, 29)));
        let unparsable = app_data.path().join("meetings.orphaned-20261301-000000");
        for dir in [&old, &recent, &unparsable] {
            fs::create_dir_all(dir.join("7")).unwrap();
            fs::write(dir.join("7").join(MIC_TRACK), b"audio").unwrap();
        }
        let old_db = app_data
            .path()
            .join(format!("meetings.db.corrupt-{}", days_ago(now, 40)));
        fs::write(&old_db, b"db").unwrap();
        fs::write(format!("{}-wal", old_db.display()), b"wal").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
        let link = app_data
            .path()
            .join(format!("meetings.corrupt-{}", days_ago(now, 31)));
        symlink(outside.path(), &link).unwrap();

        assert_eq!(purge_set_asides(app_data.path(), now), 3);

        assert!(!old.exists());
        assert!(!old_db.exists());
        assert!(recent.join("7").join(MIC_TRACK).exists());
        assert!(unparsable.join("7").join(MIC_TRACK).exists());
        assert!(fs::symlink_metadata(&link).is_ok());
        assert!(outside.path().join("keep.txt").exists());
    }

    #[test]
    fn storage_measures_the_meetings_and_what_was_set_aside_without_the_models() {
        let app_data = tempfile::tempdir().unwrap();
        let root = app_data.path().join("meetings");
        fs::create_dir_all(root.join("1")).unwrap();
        fs::create_dir_all(root.join("2")).unwrap();
        fs::create_dir_all(root.join("models")).unwrap();
        fs::write(root.join("1").join(AUDIO_FILE), vec![0; 300]).unwrap();
        fs::write(root.join("2").join(MIC_TRACK), vec![0; 50]).unwrap();
        fs::write(root.join("2").join(SYSTEM_TRACK), vec![0; 20]).unwrap();
        fs::write(root.join("models").join("model.bin"), vec![0; 9_000]).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("big.bin"), vec![0; 5_000]).unwrap();
        symlink(outside.path(), root.join("3")).unwrap();
        let aside = app_data.path().join("meetings.orphaned-20261001-101500");
        fs::create_dir_all(aside.join("4")).unwrap();
        fs::write(aside.join("4").join(AUDIO_FILE), vec![0; 40]).unwrap();
        fs::write(
            app_data.path().join("meetings.db.corrupt-20261001-101500"),
            vec![0; 6],
        )
        .unwrap();
        fs::write(app_data.path().join("meetings.db"), vec![0; 1_000]).unwrap();
        fs::write(app_data.path().join("history.db"), vec![0; 1_000]).unwrap();

        assert_eq!(
            storage(app_data.path()),
            MeetingsStorage {
                audio_bytes: 370,
                set_aside_bytes: 46,
            }
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(storage(empty.path()), MeetingsStorage::default());
    }

    #[test]
    fn a_reset_library_sets_the_recordings_aside_and_keeps_the_models() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("meetings");
        for name in ["1", "7", "3.stale-1700000000000", "models"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        fs::write(root.join("7").join(MIC_TRACK), b"audio").unwrap();
        let moved_to = parent.path().join("meetings.db.corrupt-20261017-101500");

        set_aside_recordings(&root, &moved_to).unwrap();

        let aside = parent.path().join("meetings.corrupt-20261017-101500");
        assert!(aside.join("7").join(MIC_TRACK).exists());
        assert!(aside.join("1").is_dir());
        assert!(aside.join("3.stale-1700000000000").is_dir());
        assert!(!root.join("1").exists() && !root.join("7").exists());
        assert!(!root.join("3.stale-1700000000000").exists());
        assert!(root.join("models").is_dir());
    }
}
