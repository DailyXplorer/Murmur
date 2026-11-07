use super::*;
use crate::meetings::store::{MeetingStatus, MeetingTracks};
use chrono::Utc;
use std::sync::atomic::AtomicBool;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/meetings/import_audio/fixtures")
        .join(name)
}

struct Library {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: MeetingsStore,
}

fn library() -> Library {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("meetings");
    fs::create_dir_all(&root).unwrap();
    let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), Utc::now());
    Library {
        _dir: dir,
        root,
        store,
    }
}

fn source(file: &Path) -> Source<'_> {
    Source {
        file,
        free_bytes: Some(100_000_000_000),
        now_ms: 1_700_000_000_000,
    }
}

fn entries(root: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_file_becomes_a_meeting_waiting_for_processing() {
    let library = library();
    let file = fixture("tone-44k-stereo.mp3");
    let mut shares = Vec::new();
    let id = import(
        &library.store,
        &library.root,
        &source(&file),
        &AtomicBool::new(false),
        |share| shares.push(share),
    )
    .unwrap();

    let meeting = library.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Processing);
    assert_eq!(meeting.tracks, MeetingTracks::System);
    assert_eq!(meeting.title.as_deref(), Some("tone-44k-stereo"));
    assert_eq!(meeting.started_at_ms, 1_700_000_000_000);
    assert!((300..600).contains(&meeting.duration_ms), "{meeting:?}");
    assert!(!shares.is_empty() && shares.windows(2).all(|pair| pair[0] <= pair[1]));
    // The track sits where processing looks for it; nothing else is left.
    assert!(paths::meeting_dir(&library.root, id)
        .join(SYSTEM_TRACK)
        .is_file());
    assert_eq!(entries(&library.root), [id.to_string()]);
    assert_eq!(library.store.waiting_for_processing().unwrap(), [id]);
}

#[test]
fn a_cancelled_import_leaves_nothing() {
    let library = library();
    let file = fixture("tone-44k-stereo.mp3");
    let result = import(
        &library.store,
        &library.root,
        &source(&file),
        &AtomicBool::new(true),
        |_| {},
    );
    assert_eq!(result, Err(Stopped::Cancelled));
    assert!(entries(&library.root).is_empty());
    assert!(library.store.ids().unwrap().is_empty());
}

#[test]
fn a_file_that_is_not_audio_leaves_nothing() {
    let library = library();
    let file = library.root.join("notes.mp3");
    fs::write(&file, b"this is not audio at all, only some text").unwrap();
    let result = import(
        &library.store,
        &library.root,
        &source(&file),
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(result, Err(Stopped::Failed(ImportError::Unsupported)));
    assert_eq!(entries(&library.root), ["notes.mp3"]);
    assert!(library.store.ids().unwrap().is_empty());
}

#[test]
fn a_missing_file_is_unreadable() {
    let library = library();
    let file = library.root.join("gone.wav");
    let result = import(
        &library.store,
        &library.root,
        &source(&file),
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(result, Err(Stopped::Failed(ImportError::Unreadable)));
    assert!(entries(&library.root).is_empty());
}

#[test]
fn an_import_does_not_start_on_a_full_disk() {
    let library = library();
    let file = fixture("tone-44k-stereo.mp3");
    let mut full = source(&file);
    full.free_bytes = Some(LOW_DISK_BYTES - 1);
    let result = import(
        &library.store,
        &library.root,
        &full,
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(result, Err(Stopped::Failed(ImportError::LowDisk)));
    assert!(entries(&library.root).is_empty());
    // Unknown free space does not stop it.
    full.free_bytes = None;
    assert!(import(
        &library.store,
        &library.root,
        &full,
        &AtomicBool::new(false),
        |_| {}
    )
    .is_ok());
}

#[test]
fn the_sweep_removes_abandoned_imports_and_only_those() {
    let library = library();
    let root = &library.root;
    // A crash mid-import: audio decoded so far, and the marker.
    let abandoned = create_staging(root).unwrap();
    fs::write(abandoned.join(SYSTEM_TRACK), b"half a track").unwrap();
    let second = create_staging(root).unwrap();
    // A meeting's folder, and a folder that is not an import's, even with
    // the prefix: no marker, so it is someone's.
    fs::create_dir_all(root.join("7")).unwrap();
    fs::write(root.join("7").join(SYSTEM_TRACK), b"a recording").unwrap();
    fs::create_dir_all(root.join("import-mine")).unwrap();
    fs::write(root.join("import-mine").join("a.txt"), b"keep").unwrap();
    // A file with the prefix is not a folder.
    fs::write(root.join("import-file"), b"keep").unwrap();

    assert_eq!(sweep(root), 2);
    assert!(!abandoned.exists() && !second.exists());
    assert_eq!(entries(root), ["7", "import-file", "import-mine"]);
    assert_eq!(sweep(root), 0);
}

#[test]
fn the_sweep_survives_a_missing_root() {
    let library = library();
    assert_eq!(sweep(&library.root.join("nowhere")), 0);
}

#[cfg(unix)]
#[test]
fn the_sweep_does_not_follow_a_link() {
    let library = library();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("import-1-0");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join(MARKER), b"").unwrap();
    // The meetings folder itself is a link: nothing behind it is touched.
    let linked = library._dir.path().join("linked");
    std::os::unix::fs::symlink(elsewhere.path(), &linked).unwrap();
    assert_eq!(sweep(&linked), 0);
    // A link to a folder inside the root is not followed either.
    std::os::unix::fs::symlink(&outside, library.root.join("import-link")).unwrap();
    assert_eq!(sweep(&library.root), 0);
    assert!(outside.join(MARKER).is_file());
}

#[test]
fn a_staging_folder_is_never_a_meeting_folder() {
    let library = library();
    let staging = create_staging(&library.root).unwrap();
    let name = staging.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with(STAGING_PREFIX) && name.parse::<i64>().is_err());
    assert!(staging.join(MARKER).is_file());
    // Neither of the library's own sweeps takes it for a stale or orphan
    // meeting folder.
    assert_eq!(paths::remove_stale(&library.root), 0);
    let aside = library.root.with_file_name("aside");
    assert_eq!(
        paths::set_aside_orphans(&library.root, &Default::default(), &aside).unwrap(),
        0
    );
    assert!(staging.exists());
}

#[test]
fn a_title_is_the_file_name_as_one_short_line() {
    let title = |name: &str| title_of(Path::new(name));
    assert_eq!(
        title("/a/b/Weekly sync.m4a").as_deref(),
        Some("Weekly sync")
    );
    assert_eq!(
        title("two  spaces\tand tab.wav").as_deref(),
        Some("two spaces and tab")
    );
    assert_eq!(
        title("archive.v2.final.mp3").as_deref(),
        Some("archive.v2.final")
    );
    assert_eq!(title("noext").as_deref(), Some("noext"));
    assert_eq!(title("   .mp3"), None);
    assert_eq!(title(".wav"), None);
    assert_eq!(title(".hidden.wav").as_deref(), Some(".hidden"));
    assert_eq!(
        title("left\u{202e}gnp\u{7}.wav").as_deref(),
        Some("leftgnp")
    );
    let long = format!("{}.wav", "é".repeat(300));
    assert_eq!(
        title_of(Path::new(&long)).unwrap().chars().count(),
        MAX_TITLE_CHARS
    );
}

#[test]
fn progress_is_reported_per_percent() {
    let mut seen = Vec::new();
    {
        let mut report = throttled(|share| seen.push(share));
        for step in 0..=1000 {
            report(step as f32 / 1000.0);
        }
    }
    assert_eq!(seen.first(), Some(&0.0));
    assert_eq!(seen.last(), Some(&1.0));
    assert!((95..=102).contains(&seen.len()), "{}", seen.len());
}

#[test]
fn every_decode_error_has_an_import_error() {
    use std::io::Error;
    for (decode, import) in [
        (DecodeError::Unsupported, ImportError::Unsupported),
        (DecodeError::Empty, ImportError::Empty),
        (DecodeError::TooLong, ImportError::TooLong),
        (DecodeError::Corrupt, ImportError::Corrupt),
        (
            DecodeError::Unreadable(Error::other("x")),
            ImportError::Unreadable,
        ),
        (
            DecodeError::Write(Error::from(io::ErrorKind::StorageFull)),
            ImportError::LowDisk,
        ),
        (DecodeError::Write(Error::other("x")), ImportError::Failed),
    ] {
        assert_eq!(ImportError::from(decode), import);
    }
}
