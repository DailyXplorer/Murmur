//! Delete moves a meeting to the macOS Trash instead of erasing it.
//!
//! A meeting lives in two places, its folder and `meetings.db`, so the folder
//! the Trash receives holds the notes as a Markdown file next to the audio:
//! a person can open it without Murmur. The caller deletes the rows only
//! after these functions succeed, and each of them puts every file back when
//! one of its steps fails, so a failure never loses data.
//!
//! Only a person's Delete comes through here. Audio retention, the cleanup of
//! set-aside folders and Discard erase for good, since they exist to free
//! space.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::export::{self, Transcript};
use super::paths::{self, AUDIO_FILES};
use super::store::MeetingDetail;

/// Where a deleted meeting goes. The real one is the macOS Trash; the tests
/// give a folder.
pub trait Trash {
    /// Moves `path`, a folder, to the Trash.
    fn trash(&self, path: &Path) -> io::Result<()>;
}

/// The user's Trash, through `NSFileManager`.
pub struct SystemTrash;

impl Trash for SystemTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        use objc2_foundation::{NSFileManager, NSString, NSURL};

        let url =
            NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&path.to_string_lossy()), true);
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|error| io::Error::other(error.localizedDescription().to_string()))
    }
}

/// What the Trash gets besides the audio: the notes and the name that
/// carries them.
pub struct Notes {
    /// `2026-10-17 Q4 plan.md`, the name Export suggests.
    pub file_name: String,
    pub markdown: String,
}

impl Notes {
    /// The Markdown of the meeting, summary included. `None` when its start
    /// time is not a valid date.
    pub fn of(meeting: &MeetingDetail) -> Option<Self> {
        let started_at = export::local_start(meeting.started_at_ms)?;
        let transcript = Transcript {
            title: meeting.title.as_deref(),
            started_at,
            duration_ms: meeting.duration_ms,
            speakers: &meeting.speakers,
            turns: &meeting.turns,
        };
        Some(Self {
            file_name: export::file_name(&started_at, meeting.title.as_deref()),
            markdown: export::notes_markdown(&transcript, meeting.summary.as_deref()),
        })
    }

    /// The name without its extension: the name of the folder.
    fn stem(&self) -> &str {
        self.file_name
            .strip_suffix(".md")
            .unwrap_or(&self.file_name)
    }
}

/// `stem`, or `stem (2)`, `stem (3)`… when `root` already holds it.
fn unused_path(root: &Path, stem: &str) -> PathBuf {
    let mut candidate = root.join(stem);
    let mut number = 2;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = root.join(format!("{stem} ({number})"));
        number += 1;
    }
    candidate
}

/// Puts back what `moves` (from, to) moved, newest first. A move that cannot
/// be undone is logged: nothing is lost, the file only stays where it went.
fn undo(moves: &[(PathBuf, PathBuf)]) {
    for (from, to) in moves.iter().rev() {
        if let Err(error) = fs::rename(to, from) {
            log::error!("Cannot put a meeting file back: {error}");
        }
    }
}

/// Moves meeting `id`'s folder to the Trash as `<name>/` holding `<name>.md`
/// and everything the folder held. A meeting without a folder (its audio is
/// gone) still sends its notes. The folder is renamed inside `root` first, so
/// the Trash shows the readable name and an unsafe path is never trashed.
pub fn trash_meeting(root: &Path, id: i64, notes: &Notes, trash: &dyn Trash) -> io::Result<()> {
    let (dir, created) = match paths::safe_meeting_dir(root, id)? {
        Some(dir) => (dir, false),
        None => {
            let dir = paths::meeting_dir(root, id);
            fs::create_dir_all(&dir)?;
            (dir, true)
        }
    };
    let notes_path = dir.join(&notes.file_name);
    let discard_notes = |dir: &Path| {
        let _ = fs::remove_file(&notes_path);
        if created {
            let _ = fs::remove_dir(dir);
        }
    };
    if let Err(error) = fs::write(&notes_path, &notes.markdown) {
        discard_notes(&dir);
        return Err(error);
    }
    let staged = unused_path(root, notes.stem());
    if let Err(error) = fs::rename(&dir, &staged) {
        discard_notes(&dir);
        return Err(error);
    }
    if let Err(error) = trash.trash(&staged) {
        // A Trash that failed midway may have taken it already.
        if staged.exists() {
            undo(&[(dir.clone(), staged)]);
            discard_notes(&dir);
        }
        return Err(error);
    }
    Ok(())
}

/// Moves the audio files meeting `id` still has to the Trash, in a folder
/// named `<name> audio`. The transcript stays in Murmur, so there are no
/// notes. Nothing to move is not an error.
pub fn trash_audio(root: &Path, id: i64, notes: &Notes, trash: &dyn Trash) -> io::Result<()> {
    let Some(dir) = paths::safe_meeting_dir(root, id)? else {
        return Ok(());
    };
    let present: Vec<&str> = AUDIO_FILES
        .into_iter()
        .filter(|name| fs::symlink_metadata(dir.join(name)).is_ok())
        .collect();
    if present.is_empty() {
        return Ok(());
    }
    let staged = unused_path(root, &format!("{} audio", notes.stem()));
    fs::create_dir(&staged)?;
    let mut moved = Vec::new();
    for name in present {
        let (from, to) = (dir.join(name), staged.join(name));
        if let Err(error) = fs::rename(&from, &to) {
            undo(&moved);
            let _ = fs::remove_dir(&staged);
            return Err(error);
        }
        moved.push((from, to));
    }
    if let Err(error) = trash.trash(&staged) {
        if staged.exists() {
            undo(&moved);
            let _ = fs::remove_dir(&staged);
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::meetings::paths::{AUDIO_FILE, MIC_TRACK};

    /// A folder standing in for the Trash. The real one is never touched.
    pub struct FolderTrash {
        pub bin: PathBuf,
        pub fails: bool,
    }

    impl FolderTrash {
        pub fn new(bin: &Path) -> Self {
            fs::create_dir_all(bin).unwrap();
            Self {
                bin: bin.into(),
                fails: false,
            }
        }
    }

    impl Trash for FolderTrash {
        fn trash(&self, path: &Path) -> io::Result<()> {
            if self.fails {
                return Err(io::Error::other("the Trash is full"));
            }
            fs::rename(path, self.bin.join(path.file_name().unwrap()))
        }
    }

    pub fn notes() -> Notes {
        Notes {
            file_name: "2026-10-17 Q4 plan.md".into(),
            markdown: "# Q4 plan\n".into(),
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn meeting_with(root: &Path, id: i64, files: &[&str]) -> PathBuf {
        let dir = paths::meeting_dir(root, id);
        fs::create_dir_all(&dir).unwrap();
        for file in files {
            fs::write(dir.join(file), b"audio").unwrap();
        }
        dir
    }

    #[test]
    fn the_trash_gets_a_folder_named_like_the_export_with_the_notes_and_the_audio() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let trash = FolderTrash::new(&tmp.path().join("trash"));
        let dir = meeting_with(&root, 5, &[AUDIO_FILE]);

        trash_meeting(&root, 5, &notes(), &trash).unwrap();

        let folder = trash.bin.join("2026-10-17 Q4 plan");
        assert_eq!(
            names(&folder),
            ["2026-10-17 Q4 plan.md", AUDIO_FILE].map(String::from)
        );
        assert_eq!(
            fs::read_to_string(folder.join("2026-10-17 Q4 plan.md")).unwrap(),
            "# Q4 plan\n"
        );
        assert!(!dir.exists());
        assert!(names(&root).is_empty());
    }

    #[test]
    fn a_name_already_taken_in_the_meetings_folder_gets_a_number() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let trash = FolderTrash::new(&tmp.path().join("trash"));
        meeting_with(&root, 5, &[AUDIO_FILE]);
        fs::create_dir(root.join("2026-10-17 Q4 plan")).unwrap();

        trash_meeting(&root, 5, &notes(), &trash).unwrap();

        assert!(trash.bin.join("2026-10-17 Q4 plan (2)").is_dir());
        assert!(root.join("2026-10-17 Q4 plan").is_dir());
    }

    #[test]
    fn a_failing_trash_puts_the_folder_back_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let mut trash = FolderTrash::new(&tmp.path().join("trash"));
        trash.fails = true;
        let dir = meeting_with(&root, 5, &[AUDIO_FILE, MIC_TRACK]);

        assert!(trash_meeting(&root, 5, &notes(), &trash).is_err());

        assert_eq!(names(&dir), [AUDIO_FILE, MIC_TRACK].map(String::from));
        assert_eq!(names(&root), ["5"].map(String::from));
        assert!(names(&trash.bin).is_empty());
    }

    #[test]
    fn a_meeting_whose_audio_is_gone_still_trashes_its_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let trash = FolderTrash::new(&tmp.path().join("trash"));
        meeting_with(&root, 5, &[]);

        trash_meeting(&root, 5, &notes(), &trash).unwrap();
        assert_eq!(
            names(&trash.bin.join("2026-10-17 Q4 plan")),
            ["2026-10-17 Q4 plan.md"].map(String::from)
        );

        // No folder at all, and a Trash that fails: nothing is left behind.
        let mut failing = FolderTrash::new(&tmp.path().join("other-trash"));
        failing.fails = true;
        assert!(trash_meeting(&root, 6, &notes(), &failing).is_err());
        assert!(names(&root).is_empty());
    }

    #[test]
    fn a_folder_that_is_a_link_is_never_trashed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, paths::meeting_dir(&root, 5)).unwrap();
        let trash = FolderTrash::new(&tmp.path().join("trash"));

        assert!(trash_meeting(&root, 5, &notes(), &trash).is_err());
        assert!(trash_audio(&root, 5, &notes(), &trash).is_err());

        assert_eq!(names(&outside), ["keep.txt"].map(String::from));
        assert!(names(&trash.bin).is_empty());
    }

    #[test]
    fn the_audio_alone_goes_to_the_trash_and_the_rest_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let trash = FolderTrash::new(&tmp.path().join("trash"));
        let dir = meeting_with(&root, 5, &[AUDIO_FILE, "analysis.json"]);

        trash_audio(&root, 5, &notes(), &trash).unwrap();

        assert_eq!(
            names(&trash.bin.join("2026-10-17 Q4 plan audio")),
            [AUDIO_FILE].map(String::from)
        );
        assert_eq!(names(&dir), ["analysis.json"].map(String::from));
        assert_eq!(names(&root), ["5"].map(String::from));

        // No audio left: nothing to move.
        trash_audio(&root, 5, &notes(), &trash).unwrap();
        assert_eq!(names(&trash.bin).len(), 1);
    }

    #[test]
    fn a_failing_trash_gives_the_audio_back() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meetings");
        let mut trash = FolderTrash::new(&tmp.path().join("trash"));
        trash.fails = true;
        let dir = meeting_with(&root, 5, &[AUDIO_FILE, MIC_TRACK]);

        assert!(trash_audio(&root, 5, &notes(), &trash).is_err());

        assert_eq!(names(&dir), [AUDIO_FILE, MIC_TRACK].map(String::from));
        assert_eq!(names(&root), ["5"].map(String::from));
    }

    /// Moves a real folder to the real Trash of the Mac it runs on.
    #[test]
    #[ignore = "puts a folder in the real Trash of the Mac that runs it"]
    fn the_system_trash_takes_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("murmur-trash-check");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("note.md"), b"check").unwrap();

        SystemTrash.trash(&folder).unwrap();

        assert!(!folder.exists());
    }
}
