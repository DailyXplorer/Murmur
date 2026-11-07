//! Import audio: an audio file becomes a meeting with one track, which the
//! processing a recording gets turns into a transcript, a title, a summary,
//! search and exports.
//!
//! The file is decoded into a staging folder (`import-<pid>-<n>`, beside the
//! meeting folders) first, and only a complete track is given a meeting. A
//! quit or crash during the decode, which is nearly all of an import's time,
//! therefore leaves a staging folder and no meeting; the folder carries a
//! marker and the next launch deletes it (`sweep`). A crash in the
//! milliseconds after the decode, between the meeting's row and its track
//! being in place, leaves a row that startup recovery marks failed ("no
//! audio"), which the user can delete like any other.
//!
//! The track is stored as system audio: the speaker analysis tells its
//! voices apart (`Speaker 1`, `Speaker 2`), where a microphone track is one
//! "Me".

pub mod decode;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use specta::Type;
use tauri_specta::Event;

use super::cap::CAP;
use super::coordinator::LOW_DISK_BYTES;
use super::paths::{self, SYSTEM_TRACK};
use super::rename::MAX_TITLE_CHARS;
use super::store::{MeetingTracks, MeetingsStore};
use decode::DecodeError;

/// A folder in the meetings folder with this prefix is an import in
/// progress. No meeting's folder has it: those are numbers.
pub const STAGING_PREFIX: &str = "import-";
/// The file that says a staging folder is an import's, and not someone's.
pub const MARKER: &str = "import.pending";

/// The longest audio imported: the length a recording is capped at.
pub const MAX_IMPORT: std::time::Duration = CAP;

/// Why a file was not imported. The page words each of them.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportError {
    /// Meetings are turned off.
    Disabled,
    /// An import is already running.
    Busy,
    /// The file cannot be opened.
    Unreadable,
    /// No decoder reads it.
    Unsupported,
    /// It holds no sound.
    Empty,
    /// It is longer than `MAX_IMPORT`.
    TooLong,
    /// It broke part-way.
    Corrupt,
    /// Not enough room to keep the track.
    LowDisk,
    /// The library could not take the meeting.
    Failed,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disabled => "Meetings are turned off",
            Self::Busy => "An import is already running",
            Self::Unreadable => "The file cannot be read",
            Self::Unsupported => "The format is not supported",
            Self::Empty => "The file holds no audio",
            Self::TooLong => "The audio is longer than 3 hours",
            Self::Corrupt => "The file is damaged",
            Self::LowDisk => "There is not enough free disk space",
            Self::Failed => "The meeting could not be created",
        })
    }
}

/// The import that is running, for the page.
#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingImport {
    /// The file's name, as the user knows it.
    pub file_name: String,
    /// The share of the file decoded, `0.0..=1.0`.
    pub fraction: f32,
}

/// The running import, or none. Sent whenever it starts, ends, or moves a
/// few percent.
#[derive(Serialize, Debug, Clone, PartialEq, Type, Event)]
pub struct MeetingImportChanged(pub Option<MeetingImport>);

/// What the page is told per percent: a long file would otherwise send an
/// event per packet.
const PROGRESS_STEP: f32 = 0.01;

/// A new staging folder's name: this process, and a counter.
fn staging_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{STAGING_PREFIX}{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Creates the staging folder with its marker.
fn create_staging(root: &Path) -> io::Result<PathBuf> {
    let dir = root.join(staging_name());
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(MARKER), b"")?;
    Ok(dir)
}

/// Deletes the staging folders an earlier run left (a quit or a crash in the
/// middle of an import). No import runs yet at launch. A folder without the
/// marker is not ours and stays. Returns how many went.
pub fn sweep(root: &Path) -> usize {
    // Inside a linked root, a deletion would act outside the app's data.
    if fs::symlink_metadata(root).is_ok_and(|root| root.file_type().is_symlink()) {
        return 0;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let is_staging = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        // `file_type` of a symlink is the link's, so a link is never followed.
        if !is_staging || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if !entry.path().join(MARKER).is_file() {
            continue;
        }
        match fs::remove_dir_all(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) => log::warn!(
                "Cannot delete the abandoned import {}: {error}",
                entry.path().display()
            ),
        }
    }
    removed
}

/// A meeting's title from the file it was made from: the name without its
/// extension, as one line, short enough to be a title. `None` when nothing
/// is left of it.
pub fn title_of(path: &Path) -> Option<String> {
    // `.wav` is a hidden file called "wav", not a title.
    let name = path.file_name()?.to_string_lossy();
    if name.starts_with('.') && !name[1..].contains('.') {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy();
    // Control characters and the bidirectional overrides would show the
    // title reversed or garbled in the list and in exports.
    let clean: String = stem
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect();
    let line = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let title: String = line.chars().take(MAX_TITLE_CHARS).collect();
    let title = title.trim_end().to_string();
    (!title.is_empty()).then_some(title)
}

/// The largest track an import writes: `MAX_IMPORT` of 16 kHz 16-bit mono.
const MAX_TRACK_BYTES: u64 = 3 * 3600 * 16_000 * 2;

// The low-disk threshold covers the largest track, so a start above it
// cannot fill the disk with the decode.
const _: () = assert!(MAX_TRACK_BYTES < LOW_DISK_BYTES);

fn no_room(free_bytes: Option<u64>) -> bool {
    free_bytes.is_some_and(|free| free < LOW_DISK_BYTES)
}

impl From<DecodeError> for ImportError {
    fn from(error: DecodeError) -> Self {
        match error {
            DecodeError::Unreadable(_) => Self::Unreadable,
            DecodeError::Unsupported => Self::Unsupported,
            DecodeError::Empty => Self::Empty,
            DecodeError::TooLong => Self::TooLong,
            DecodeError::Corrupt => Self::Corrupt,
            DecodeError::Write(error) if error.kind() == io::ErrorKind::StorageFull => {
                Self::LowDisk
            }
            DecodeError::Write(_) => Self::Failed,
            // The caller checks `cancel` before it asks why.
            DecodeError::Cancelled => Self::Failed,
        }
    }
}

/// What an import needs from the outside.
pub struct Source<'a> {
    pub file: &'a Path,
    /// Free bytes where meetings are kept, when known.
    pub free_bytes: Option<u64>,
    /// When the meeting is dated.
    pub now_ms: i64,
}

/// How an import ended other than by making a meeting.
#[derive(Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The user cancelled.
    Cancelled,
    Failed(ImportError),
}

impl From<ImportError> for Stopped {
    fn from(error: ImportError) -> Self {
        Self::Failed(error)
    }
}

/// Decodes `source.file` and makes a meeting of it, waiting for processing.
/// `progress` gets the share decoded; `cancel` set ends the import and leaves
/// nothing behind. Returns the meeting's id.
///
/// The meeting is dated when it is imported, not when the file was made: the
/// audio retention counts from the date, and a recording from last year
/// would lose its audio the day it is imported. Its title is the file's
/// name until the summary writes one.
pub fn import(
    store: &MeetingsStore,
    root: &Path,
    source: &Source<'_>,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<i64, Stopped> {
    if no_room(source.free_bytes) {
        return Err(ImportError::LowDisk.into());
    }
    let staging = create_staging(root).map_err(|error| {
        log::error!("Import: cannot create the staging folder: {error}");
        ImportError::Failed
    })?;
    let result = import_from(store, root, source, &staging, cancel, progress);
    if let Err(error) = fs::remove_dir_all(&staging) {
        // The next launch's sweep takes it.
        log::warn!("Import: cannot delete {}: {error}", staging.display());
    }
    result
}

fn import_from(
    store: &MeetingsStore,
    root: &Path,
    source: &Source<'_>,
    staging: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(f32),
) -> Result<i64, Stopped> {
    let track = staging.join(SYSTEM_TRACK);
    let decoded = decode::decode_to_wav(
        source.file,
        &track,
        u64::try_from(MAX_IMPORT.as_millis()).unwrap_or(u64::MAX),
        cancel,
        progress,
    );
    if cancel.load(Ordering::Relaxed) {
        return Err(Stopped::Cancelled);
    }
    let duration_ms = i64::try_from(decoded.map_err(ImportError::from)?.duration_ms())
        .map_err(|_| ImportError::Failed)?;
    let title = title_of(source.file);
    let id = store
        .insert_import(source.now_ms, MeetingTracks::System, title.as_deref())
        .map_err(|error| {
            log::error!("Import: cannot add the meeting: {error}");
            ImportError::Failed
        })?;
    let moved = paths::fresh_meeting_dir(root, id, source.now_ms)
        .and_then(|dir| fs::rename(&track, dir.join(SYSTEM_TRACK)))
        .map_err(|error| {
            log::error!("Import: cannot keep the track of meeting {id}: {error}");
            ImportError::Failed
        })
        .and_then(|()| {
            store
                .finish_recording(id, source.now_ms + duration_ms, duration_ms)
                .map_err(|error| {
                    log::error!("Import: cannot finish meeting {id}: {error}");
                    ImportError::Failed
                })
        });
    if let Err(error) = moved {
        // No row, no folder: nothing for the library to show or to fail on.
        let _ = store.delete(id);
        let _ = paths::remove_meeting_dir(root, id);
        return Err(error.into());
    }
    Ok(id)
}

/// Reports `fraction` only when it moved `PROGRESS_STEP` since the last one.
pub fn throttled(mut report: impl FnMut(f32)) -> impl FnMut(f32) {
    let mut last = -1.0_f32;
    move |fraction| {
        if fraction >= 1.0 || fraction - last >= PROGRESS_STEP {
            last = fraction;
            report(fraction);
        }
    }
}

#[cfg(test)]
mod tests;
