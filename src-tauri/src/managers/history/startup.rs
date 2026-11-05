use anyhow::Result;
use chrono::{DateTime, Utc};
use log::error;
use std::fs;
use std::path::{Path, PathBuf};

/// What opening a database at startup produced.
#[derive(Debug, PartialEq, Eq)]
pub enum DatabaseStartup {
    Ready,
    /// The file was not a readable database. It was renamed to this path and
    /// an empty database replaced it.
    Reset {
        moved_to: PathBuf,
    },
    /// The database is unusable this session (for example, the file is
    /// locked). Dictation still works; saves fail and are logged.
    Unavailable,
}

/// Opens and migrates the database with `init`. A file SQLite cannot read as a
/// database is renamed to `<name>.corrupt-<timestamp>` and replaced by an
/// empty one.
pub(crate) fn open_or_reset_database(
    db_path: &Path,
    now: DateTime<Utc>,
    init: impl Fn(&Path) -> Result<()>,
) -> DatabaseStartup {
    let name = db_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let first_error = match init(db_path) {
        Ok(()) => return DatabaseStartup::Ready,
        Err(error) => error,
    };
    if !is_corruption(&first_error) {
        error!("Database {name} is unavailable: {first_error:#}");
        return DatabaseStartup::Unavailable;
    }

    let moved_to =
        db_path.with_file_name(format!("{name}.corrupt-{}", now.format("%Y%m%d-%H%M%S")));
    error!(
        "Database {name} is corrupt ({first_error:#}); moving it to {}",
        moved_to.display()
    );
    if let Err(error) = fs::rename(db_path, &moved_to) {
        error!("Failed to move the corrupt database {name} aside: {error}");
        return DatabaseStartup::Unavailable;
    }
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if sidecar.exists() {
            let _ = fs::rename(
                &sidecar,
                PathBuf::from(format!("{}{suffix}", moved_to.display())),
            );
        }
    }
    match init(db_path) {
        Ok(()) => DatabaseStartup::Reset { moved_to },
        Err(error) => {
            error!("Failed to recreate the database {name}: {error:#}");
            DatabaseStartup::Unavailable
        }
    }
}

/// After `history.db` was found corrupt and moved to `moved_to`, moves the
/// dictation recordings next to it, to `recordings.corrupt-<time>`: no row
/// points to them any more, so retention would never delete them, and the
/// history player could still read them. A new folder replaces them.
pub(super) fn set_aside_recordings(recordings_dir: &Path, moved_to: &Path) -> std::io::Result<()> {
    if !recordings_dir.exists() {
        return Ok(());
    }
    let suffix = moved_to
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("history.db"))
        .unwrap_or(".corrupt");
    fs::rename(
        recordings_dir,
        recordings_dir.with_file_name(format!("recordings{suffix}")),
    )?;
    fs::create_dir_all(recordings_dir)
}

fn is_corruption(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let sqlite_error = cause.downcast_ref::<rusqlite::Error>().or_else(|| {
            match cause.downcast_ref::<rusqlite_migration::Error>() {
                Some(rusqlite_migration::Error::RusqliteError { err, .. }) => Some(err),
                _ => None,
            }
        });
        matches!(
            sqlite_error.and_then(rusqlite::Error::sqlite_error_code),
            Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
        )
    })
}
