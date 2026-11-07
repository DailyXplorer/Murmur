mod entries;
mod recordings;
mod retention;
mod schema;
mod startup;
#[cfg(test)]
mod tests;

pub(crate) use schema::{init_database, migrations};
pub(crate) use startup::open_or_reset_database;
pub use startup::DatabaseStartup;

use anyhow::Result;
use chrono::Utc;
use log::{debug, error};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use self::startup::set_aside_recordings;
use crate::cleanup::CleanupOutcome;
use crate::engine::{Engine, FallbackReason};

pub const RECORDING_UNAVAILABLE_ERROR: &str = "Recording unavailable";

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
    /// Every dictation that was not starred was removed at once ("Clear
    /// history"); starred ones remain.
    #[serde(rename = "cleared")]
    Cleared,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    /// The text that was delivered.
    pub transcription_text: String,
    /// The transcript before AI cleanup, when cleanup ran.
    pub raw_text: Option<String>,
    pub cleanup: Option<CleanupOutcome>,
    pub engine: Engine,
    /// Set when the engine is not the one the user chose.
    pub fallback_reason: Option<FallbackReason>,
}

/// A dictation that matched a search.
#[derive(Clone, Debug, Serialize, Type)]
pub struct HistoryHit {
    pub entry: HistoryEntry,
    /// Where `entry.transcription_text` matched, in UTF-16 units.
    pub ranges: Vec<(u32, u32)>,
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

impl HistoryManager {
    /// Never fails on a broken database: it is moved aside or, failing that,
    /// reported through `DatabaseStartup` so the app keeps running.
    pub fn new(app_handle: &AppHandle) -> Result<(Self, DatabaseStartup)> {
        let app_data_dir = app_handle.path().app_data_dir()?;
        #[cfg(not(target_os = "windows"))]
        let recordings_dir = app_data_dir.join("recordings");
        #[cfg(target_os = "windows")]
        let recordings_dir = crate::data_dirs_windows::recordings_dir(&app_data_dir);
        let db_path = app_data_dir.join("history.db");

        if !recordings_dir.exists() {
            if let Err(error) = fs::create_dir_all(&recordings_dir) {
                error!("Failed to create recordings directory: {error}");
            } else {
                debug!("Created recordings directory: {:?}", recordings_dir);
            }
        }

        let startup = open_or_reset_database(&db_path, Utc::now(), init_database);
        if let DatabaseStartup::Reset { moved_to } = &startup {
            if let Err(error) = set_aside_recordings(&recordings_dir, moved_to) {
                error!("Failed to set aside the recordings of the reset history: {error}");
            }
        }
        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };
        Ok((manager, startup))
    }

    fn get_connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        // The retention sweep writes from its own thread; wait for it rather
        // than failing a save with SQLITE_BUSY.
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(conn)
    }
}
