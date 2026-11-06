//! The live transcript on disk: one JSON entry per line in the meeting's
//! folder, appended as sentences are published, so a crash keeps what was
//! shown. Deleted once the final transcript is saved, after which nothing
//! appends to it again.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::LiveEntry;

pub const DRAFT_FILE: &str = "live.ndjson";

/// The folders whose draft was deleted. The live worker outlives Stop: it
/// can publish its last sentences after the processor saved the final
/// transcript, and such an append would bring the file back. Appending and
/// deleting share this lock, so one never lands after the other's check.
static SEALED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn sealed() -> MutexGuard<'static, Option<HashSet<PathBuf>>> {
    SEALED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A meeting starts recording in `dir`: its draft may be written again.
pub fn reopen(dir: &Path) {
    if let Some(folders) = sealed().as_mut() {
        folders.remove(dir);
    }
}

pub fn append(dir: &Path, entries: &[LiveEntry]) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let sealed = sealed();
    if sealed.as_ref().is_some_and(|folders| folders.contains(dir)) {
        return Ok(());
    }
    let mut lines = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut lines, entry)?;
        lines.push(b'\n');
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(DRAFT_FILE))?
        .write_all(&lines)
}

/// The entries saved, `None` without a draft. A line a crash cut off is
/// skipped, even when it ends inside a multi-byte character.
pub fn read(dir: &Path) -> Option<Vec<LiveEntry>> {
    let bytes = std::fs::read(dir.join(DRAFT_FILE)).ok()?;
    Some(
        bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice(line).ok())
            .collect(),
    )
}

/// Deletes the draft for good: later appends to `dir` do nothing.
pub fn remove(dir: &Path) -> io::Result<()> {
    sealed()
        .get_or_insert_with(HashSet::new)
        .insert(dir.to_path_buf());
    match std::fs::remove_file(dir.join(DRAFT_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::live::{LiveEntryKind, Source};

    #[test]
    fn a_late_append_does_not_bring_back_a_deleted_draft() {
        let dir = tempfile::tempdir().unwrap();
        let entry = |text: &str| LiveEntry {
            source: Source::Mic,
            start_ms: 0,
            end_ms: 1_000,
            text: text.into(),
            kind: LiveEntryKind::Speech,
        };
        append(dir.path(), &[entry("Before.")]).unwrap();
        remove(dir.path()).unwrap();

        append(dir.path(), &[entry("After.")]).unwrap();
        assert_eq!(read(dir.path()), None);

        reopen(dir.path());
        append(dir.path(), &[entry("Again.")]).unwrap();
        assert_eq!(read(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_line_cut_inside_a_character_costs_only_that_line() {
        let dir = tempfile::tempdir().unwrap();
        let entry = |text: &str| LiveEntry {
            source: Source::Mic,
            start_ms: 0,
            end_ms: 1_000,
            text: text.into(),
            kind: LiveEntryKind::Speech,
        };
        append(dir.path(), &[entry("Déjà dit."), entry("Voilà.")]).unwrap();
        let mut torn = br#"{"source":"mic","start_ms":0,"end_ms":1000,"text":"Cr"#.to_vec();
        torn.push("é".as_bytes()[0]);
        OpenOptions::new()
            .append(true)
            .open(dir.path().join(DRAFT_FILE))
            .unwrap()
            .write_all(&torn)
            .unwrap();

        let texts: Vec<_> = read(dir.path())
            .unwrap()
            .into_iter()
            .map(|entry| entry.text)
            .collect();
        assert_eq!(texts, ["Déjà dit.", "Voilà."]);
    }
}
