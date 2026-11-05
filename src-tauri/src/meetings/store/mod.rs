//! `meetings.db`: one row per meeting, plus the chunk, speaker and turn tables
//! that processing fills in later. Migrations are append-only, as in
//! `managers::history`.
//!
//! Layout: `audio`, `library`, `migrations`, `processing`, `search`, `summary`, `transcript`, `types`.

mod audio;
mod library;
mod migrations;
mod processing;
mod search;
mod summary;
#[cfg(test)]
mod tests;
mod transcript;
mod types;

pub use library::list_page;
pub(crate) use migrations::migrations;
pub use search::{search, with_speaker_names};
pub use types::{
    AudioState, ChunkRow, ChunkStatus, MeetingDetail, MeetingHit, MeetingSpeaker, MeetingStage,
    MeetingStatus, MeetingTracks, MeetingTurn, MeetingsPage, ProcessingError, SpeakerAnalysis,
    SummaryStatus,
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::managers::history::{open_or_reset_database, DatabaseStartup};

pub struct MeetingsStore {
    db_path: PathBuf,
}

fn init_database(db_path: &Path) -> Result<()> {
    let mut conn = Connection::open(db_path)?;
    // Reading the schema fails fast on a file that is not a database.
    let integrity: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            Some(integrity),
        )
        .into());
    }
    migrations().to_latest(&mut conn)?;
    Ok(())
}

impl MeetingsStore {
    /// Never fails: a corrupt file is moved aside, and an unusable one makes
    /// every later call return an error.
    pub fn open(db_path: PathBuf, now: DateTime<Utc>) -> (Self, DatabaseStartup) {
        let startup = open_or_reset_database(&db_path, now, init_database);
        (Self { db_path }, startup)
    }

    fn connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(conn)
    }
}
