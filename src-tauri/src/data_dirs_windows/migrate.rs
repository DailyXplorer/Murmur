//! The one-time move of Murmur's heavy folders (`models`, `meetings`,
//! `recordings`) from the Roaming profile to Local app data.
//!
//! Roaming profiles sync to a server at sign-in and sign-out, so a 740 MB
//! model and hours of meeting audio do not belong there. The small files
//! (`settings_store.json`, `history.db`, `meetings.db`) stay in Roaming:
//! `murmur mcp` and the docs read them from there.
//!
//! The rules, in order of importance:
//!
//! 1. Nothing is deleted before everything it holds is in the new place with
//!    the same length.
//! 2. A move that cannot finish (a locked file, a full disk, a share that
//!    went away) keeps the old folder in use and is tried again at the next
//!    launch.
//! 3. Every step can be repeated: a launch killed halfway leaves a new folder
//!    holding some complete files, and the next launch carries on from there.
//!    A file reaches its final name only once fully written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use log::{info, warn};

/// A file being copied lives under this extra suffix until it is complete.
const PART_SUFFIX: &str = ".moving";

/// Tries to remove a file that is still held by another process (a virus
/// scanner, the search indexer) before leaving it for the next launch.
const REMOVE_ATTEMPTS: u32 = 3;
const REMOVE_PAUSE: Duration = Duration::from_millis(100);

/// Where each heavy folder is used from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    roaming: PathBuf,
    models: PathBuf,
    meetings: PathBuf,
    recordings: PathBuf,
}

impl Layout {
    /// Everything in Roaming, as before the move existed.
    pub fn in_roaming(roaming: &Path) -> Self {
        Self {
            roaming: roaming.to_path_buf(),
            models: roaming.join("models"),
            meetings: roaming.join("meetings"),
            recordings: roaming.join("recordings"),
        }
    }

    pub fn roaming(&self) -> &Path {
        &self.roaming
    }

    /// The folder that holds `models`.
    pub fn models_parent(&self) -> &Path {
        parent_of(&self.models, &self.roaming)
    }

    pub fn meetings(&self) -> &Path {
        &self.meetings
    }

    pub fn recordings(&self) -> &Path {
        &self.recordings
    }

    /// The folders besides Roaming where things set aside by the meetings
    /// library or the dictation history land (they sit next to the folder
    /// they come from).
    pub fn set_aside_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for folder in [&self.meetings, &self.recordings] {
            let parent = parent_of(folder, &self.roaming);
            if parent != self.roaming && !dirs.iter().any(|dir| dir == parent) {
                dirs.push(parent.to_path_buf());
            }
        }
        dirs
    }
}

fn parent_of<'a>(folder: &'a Path, fallback: &'a Path) -> &'a Path {
    folder.parent().unwrap_or(fallback)
}

/// Moves each heavy folder from `roaming` to `local` and says where each is
/// used from. With `roaming` equal to `local` nothing moves.
pub fn resolve(roaming: &Path, local: &Path) -> Layout {
    let mut layout = Layout::in_roaming(roaming);
    if roaming == local {
        return layout;
    }
    for (name, slot) in [
        ("models", &mut layout.models),
        ("meetings", &mut layout.meetings),
        ("recordings", &mut layout.recordings),
    ] {
        let (old, new) = (roaming.join(name), local.join(name));
        match move_folder(&old, &new) {
            Moved::Done => *slot = new,
            Moved::Kept(reason) => warn!(
                "Keeping {} in {}, to move it at the next launch: {reason}",
                name,
                roaming.display()
            ),
        }
    }
    layout
}

#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    /// `new` holds the folder (the old one is gone, or only holds what could
    /// not be deleted or what `new` already has a newer version of).
    Done,
    /// Keep using `old`; the reason is for the log.
    Kept(String),
}

/// What happened to a file in the old folder.
#[derive(Debug, PartialEq, Eq)]
enum FileState {
    /// Copied now.
    Copied,
    /// Already in the new folder with the same length.
    Present,
    /// The new folder holds a different, newer file of that name (or
    /// something that is not a file): the old one stays where it is.
    Conflict,
}

#[derive(Default)]
struct Report {
    /// Files of the old folder that the new folder now holds.
    verified: Vec<PathBuf>,
    /// Links and files kept in the old folder on purpose.
    left: usize,
    /// Things that could not be read or written.
    failed: usize,
}

/// Moves the folder `old` to `new`. See the module notes for the rules.
pub fn move_folder(old: &Path, new: &Path) -> Moved {
    match fs::symlink_metadata(old) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Moved::Done,
        Err(error) => return Moved::Kept(format!("cannot read {}: {error}", old.display())),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Moved::Kept(format!("{} is not a plain folder", old.display()))
        }
        Ok(_) => {}
    }

    match fs::symlink_metadata(new) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = new.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    return Moved::Kept(format!("cannot create {}: {error}", parent.display()));
                }
            }
            if rename_works(old, new) {
                return Moved::Done;
            }
        }
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            // An empty folder is in the way of a rename only.
            if fs::remove_dir(new).is_ok() && rename_works(old, new) {
                return Moved::Done;
            }
        }
        Ok(_) => return Moved::Kept(format!("{} is not a plain folder", new.display())),
        Err(error) => return Moved::Kept(format!("cannot read {}: {error}", new.display())),
    }

    let mut report = Report::default();
    merge_dir(old, new, &mut report);
    if report.failed > 0 {
        return Moved::Kept(format!(
            "{} item(s) could not be copied; nothing was deleted",
            report.failed
        ));
    }

    let mut stuck = 0;
    for file in &report.verified {
        if !remove_file(file) {
            stuck += 1;
        }
    }
    remove_empty_dirs(old);
    info!(
        "Moved {} to {} ({} file(s) copied or already there, {stuck} not deleted, {} left)",
        old.display(),
        new.display(),
        report.verified.len(),
        report.left
    );
    Moved::Done
}

/// A rename is the whole move when the two folders are on one volume and
/// nothing inside is open. Anything else falls back to copying.
fn rename_works(old: &Path, new: &Path) -> bool {
    match fs::rename(old, new) {
        Ok(()) => true,
        Err(error) => {
            info!(
                "Cannot rename {} to {} ({error}); copying instead",
                old.display(),
                new.display()
            );
            false
        }
    }
}

/// Copies what `old` holds that `new` lacks, folder by folder.
fn merge_dir(old: &Path, new: &Path, report: &mut Report) {
    if let Err(error) = fs::create_dir_all(new) {
        warn!("Cannot create {}: {error}", new.display());
        report.failed += 1;
        return;
    }
    let entries = match fs::read_dir(old) {
        Ok(entries) => entries,
        Err(error) => {
            warn!("Cannot list {}: {error}", old.display());
            report.failed += 1;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            report.failed += 1;
            continue;
        };
        let (from, to) = (entry.path(), new.join(entry.file_name()));
        let Ok(kind) = entry.file_type() else {
            report.failed += 1;
            continue;
        };
        if kind.is_symlink() {
            warn!("Leaving the link {} where it is", from.display());
            report.left += 1;
        } else if kind.is_dir() {
            merge_dir(&from, &to, report);
        } else if kind.is_file() {
            match copy_file(&from, &to) {
                Ok(FileState::Copied | FileState::Present) => report.verified.push(from),
                Ok(FileState::Conflict) => {
                    warn!(
                        "{} differs from the newer {}; leaving it",
                        from.display(),
                        to.display()
                    );
                    report.left += 1;
                }
                Err(error) => {
                    warn!("Cannot copy {}: {error}", from.display());
                    report.failed += 1;
                }
            }
        } else {
            report.left += 1;
        }
    }
}

/// Copies `from` to `to` through a part file, then checks the length.
fn copy_file(from: &Path, to: &Path) -> io::Result<FileState> {
    let source = fs::metadata(from)?;
    match fs::symlink_metadata(to) {
        Ok(existing) if existing.is_file() && existing.len() == source.len() => {
            return Ok(FileState::Present)
        }
        Ok(existing) if existing.is_file() => {
            // The old folder was in use after an earlier launch started to
            // copy: the file there is the newer one. Otherwise the new folder
            // was in use, and its file wins.
            let source_is_newer = matches!(
                (source.modified(), existing.modified()),
                (Ok(source_time), Ok(existing_time)) if source_time > existing_time
            );
            if !source_is_newer {
                return Ok(FileState::Conflict);
            }
        }
        Ok(_) => return Ok(FileState::Conflict),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut part_name = to.file_name().unwrap_or_default().to_os_string();
    part_name.push(PART_SUFFIX);
    let part = to.with_file_name(part_name);
    let finish = || -> io::Result<()> {
        fs::copy(from, &part)?;
        check_length(&part, source.len())?;
        fs::rename(&part, to)?;
        check_length(to, source.len())
    };
    match finish() {
        Ok(()) => Ok(FileState::Copied),
        Err(error) => {
            let _ = fs::remove_file(&part);
            Err(error)
        }
    }
}

fn check_length(path: &Path, expected: u64) -> io::Result<()> {
    let found = fs::metadata(path)?.len();
    if found == expected {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} is {found} bytes long, expected {expected}",
            path.display()
        )))
    }
}

/// Deletes a copied file. A file another process holds is retried briefly,
/// then left for the next launch.
fn remove_file(path: &Path) -> bool {
    for attempt in 1..=REMOVE_ATTEMPTS {
        match fs::remove_file(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
            Err(error) => {
                if attempt == REMOVE_ATTEMPTS {
                    warn!("Cannot delete {}: {error}", path.display());
                } else {
                    thread::sleep(REMOVE_PAUSE);
                }
            }
        }
    }
    false
}

/// Removes `dir` and every folder under it that holds nothing. A folder with
/// anything in it, and every link, stays.
fn remove_empty_dirs(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                remove_empty_dirs(&entry.path());
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

#[cfg(test)]
mod tests;
