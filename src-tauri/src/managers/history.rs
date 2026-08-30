use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

pub const RECORDING_UNAVAILABLE_ERROR: &str = "Recording unavailable";

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up(
        "ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;",
    ),
    M::up(
        "CREATE TABLE transcription_history_cloud_only (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );
        INSERT INTO transcription_history_cloud_only (
            id, file_name, timestamp, saved, title, transcription_text
        ) SELECT id, file_name, timestamp, saved, title, transcription_text
          FROM transcription_history;
        DROP TABLE transcription_history;
        ALTER TABLE transcription_history_cloud_only RENAME TO transcription_history;",
    ),
];

/// Converts the migration counter used by the former SQL plugin into SQLite's
/// `user_version` so already-applied schema changes are not replayed.
fn migrate_from_tauri_plugin_sql(conn: &Connection) -> Result<()> {
    let has_legacy_tracking: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_legacy_tracking {
        return Ok(());
    }

    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > 0 {
        return Ok(());
    }

    let legacy_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if legacy_version > 0 {
        info!(
            "Converting legacy history migration tracking at version {}",
            legacy_version
        );
        conn.pragma_update(None, "user_version", legacy_version)?;
    }

    Ok(())
}

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
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

impl HistoryManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let app_data_dir = app_handle.path().app_data_dir()?;
        let recordings_dir = app_data_dir.join("recordings");
        let db_path = app_data_dir.join("history.db");

        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        migrate_from_tauri_plugin_sql(&conn)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!("Database version before migration: {}", version_before);

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version_after > version_before {
            info!(
                "Database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!("Database already at latest version {}", version_after);
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
        })
    }

    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    fn unavailable_recording_error() -> anyhow::Error {
        anyhow!(RECORDING_UNAVAILABLE_ERROR)
    }

    fn resolve_recording_path_in_dir(recordings_dir: &Path, file_name: &str) -> Result<PathBuf> {
        let path = Path::new(file_name);
        let mut components = path.components();
        let has_one_normal_component =
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();

        if file_name.is_empty()
            || file_name.starts_with('.')
            || file_name.contains(['/', '\\'])
            || !file_name.ends_with(".wav")
            || !has_one_normal_component
        {
            return Err(Self::unavailable_recording_error());
        }

        let canonical_root =
            fs::canonicalize(recordings_dir).map_err(|_| Self::unavailable_recording_error())?;
        let candidate = recordings_dir.join(file_name);
        let metadata =
            fs::symlink_metadata(&candidate).map_err(|_| Self::unavailable_recording_error())?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Self::unavailable_recording_error());
        }

        let canonical_candidate =
            fs::canonicalize(candidate).map_err(|_| Self::unavailable_recording_error())?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(Self::unavailable_recording_error());
        }

        Ok(canonical_candidate)
    }

    pub fn resolve_persisted_recording_path(&self, file_name: &str) -> Result<PathBuf> {
        Self::resolve_recording_path_in_dir(&self.recordings_dir, file_name)
    }

    fn delete_recording_file(recordings_dir: &Path, file_name: &str) -> bool {
        let Ok(file_path) = Self::resolve_recording_path_in_dir(recordings_dir, file_name) else {
            return false;
        };

        if fs::remove_file(file_path).is_err() {
            error!("Failed to delete history recording");
            return false;
        }

        true
    }

    /// Save a new history entry to the database.
    /// The WAV file should already have been written to the recordings directory.
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&file_name, timestamp, false, &title, &transcription_text,],
        )?;

        let entry = HistoryEntry {
            id: conn.last_insert_rowid(),
            file_name,
            timestamp,
            saved: false,
            title,
            transcription_text,
        };

        debug!("Saved history entry with id {}", entry.id);

        self.cleanup_old_entries()?;

        // Emit typed event for real-time frontend updates
        if let Err(e) = (HistoryUpdatePayload::Added {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription(
        &self,
        id: i64,
        transcription_text: String,
    ) -> Result<HistoryEntry> {
        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE transcription_history SET transcription_text = ?1 WHERE id = ?2",
            params![transcription_text, id],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = conn.query_row(
            "SELECT id, file_name, timestamp, saved, title, transcription_text
                 FROM transcription_history WHERE id = ?1",
            params![id],
            Self::map_history_entry,
        )?;

        debug!("Updated transcription for history entry {}", id);

        if let Err(e) = (HistoryUpdatePayload::Updated {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period = crate::settings::get_recording_retention_period(&self.app_handle);

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Use the old count-based logic with history_limit
                let limit = crate::settings::get_history_limit(&self.app_handle);
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    fn delete_entries_and_files_with_conn(
        conn: &Connection,
        recordings_dir: &Path,
        entries: &[(i64, String)],
    ) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let mut deleted_count = 0;

        for (id, file_name) in entries {
            conn.execute(
                "DELETE FROM transcription_history WHERE id = ?1",
                params![id],
            )?;

            if Self::delete_recording_file(recordings_dir, file_name) {
                deleted_count += 1;
            }
        }

        Ok(deleted_count)
    }

    fn delete_entries_and_files(&self, entries: &[(i64, String)]) -> Result<usize> {
        let conn = self.get_connection()?;
        Self::delete_entries_and_files_with_conn(&conn, &self.recordings_dir, entries)
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

        // Get all entries that are not saved, ordered by timestamp desc
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries.push(row?);
        }

        if entries.len() > limit {
            let entries_to_delete = &entries[limit..];
            let deleted_count = self.delete_entries_and_files(entries_to_delete)?;

            if deleted_count > 0 {
                debug!("Cleaned up {} old history entries by count", deleted_count);
            }
        }

        Ok(())
    }

    fn cleanup_by_time(
        &self,
        retention_period: crate::settings::RecordingRetentionPeriod,
    ) -> Result<()> {
        let conn = self.get_connection()?;

        // Calculate cutoff timestamp (current time minus retention period)
        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60), // 3 days in seconds
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60), // 2 weeks in seconds
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60), // 3 months in seconds (approximate)
            _ => unreachable!("Should not reach here"),
        };

        // Get all unsaved entries older than the cutoff timestamp
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
        )?;

        let rows = stmt.query_map(params![cutoff_timestamp], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries_to_delete: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;

        if deleted_count > 0 {
            debug!(
                "Cleaned up {} old history entries based on retention period",
                deleted_count
            );
        }

        Ok(())
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text
                     FROM transcription_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Toggled { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    pub async fn get_audio_file_path(&self, id: i64) -> Result<PathBuf> {
        let entry = self
            .get_entry_by_id(id)
            .await?
            .ok_or_else(Self::unavailable_recording_error)?;

        self.resolve_persisted_recording_path(&entry.file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text
             FROM transcription_history
             WHERE id = ?1",
        )?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    fn delete_entry_with_conn(conn: &Connection, recordings_dir: &Path, id: i64) -> Result<()> {
        let file_name = conn
            .query_row(
                "SELECT file_name FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>("file_name"),
            )
            .optional()?;

        if let Some(file_name) = file_name {
            Self::delete_recording_file(recordings_dir, &file_name);
        }

        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        Ok(())
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        Self::delete_entry_with_conn(&conn, &self.recordings_dir, id)?;

        debug!("Deleted history entry with id: {}", id);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL
            );",
        )
        .expect("create transcription_history table");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                format!("murmur-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
            ],
        )
        .expect("insert history entry");
    }

    fn insert_entry_with_file_name(conn: &Connection, file_name: &str) -> i64 {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![file_name, 100, false, "Recording 100", "transcription"],
        )
        .expect("insert history entry");
        conn.last_insert_rowid()
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first");
        insert_entry(&conn, 200, "second");

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed");
        insert_entry(&conn, 200, "");

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
    }

    #[test]
    fn frontend_requests_audio_by_history_entry_id() {
        let frontend = include_str!("../../../src/components/settings/history/HistorySettings.tsx");

        assert!(frontend.contains("const getAudioUrl = useCallback(async (id: number)"));
        assert!(frontend.contains("commands.getAudioFilePath(id)"));
        assert!(!frontend.contains("getAudioUrl(entry.file_name)"));
    }

    #[test]
    fn asset_protocol_allows_only_literal_recording_wavs() {
        let config = include_str!("../../tauri.conf.json");

        assert!(config.contains("\"allow\": [\"$APPDATA/recordings/*.wav\"]"));
        assert!(config.contains("\"requireLiteralLeadingDot\": true"));
    }

    #[test]
    fn resolves_existing_root_level_wav_inside_canonical_recordings_directory() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        let recording = recordings_dir.join("murmur-100.wav");
        std::fs::write(&recording, "wav").expect("write recording");

        let resolved =
            HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "murmur-100.wav")
                .expect("resolve recording");

        assert_eq!(
            resolved,
            std::fs::canonicalize(&recording).expect("canonicalize recording")
        );
        assert!(resolved.starts_with(
            std::fs::canonicalize(&recordings_dir).expect("canonicalize recordings directory")
        ));
    }

    #[test]
    fn rejects_invalid_recording_file_names() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        std::fs::create_dir(recordings_dir.join("directory.wav"))
            .expect("create recording directory");

        let absolute_path = temp.path().join("outside.wav");
        let invalid_names = vec![
            String::new(),
            ".".to_string(),
            "..".to_string(),
            "../outside.wav".to_string(),
            absolute_path.display().to_string(),
            "nested/recording.wav".to_string(),
            r"nested\recording.wav".to_string(),
            ".hidden.wav".to_string(),
            "recording.WAV".to_string(),
            "recording.mp3".to_string(),
            "missing.wav".to_string(),
            "directory.wav".to_string(),
        ];

        for file_name in invalid_names {
            let error = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, &file_name)
                .expect_err("reject invalid recording name");
            assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_external_recording_symlinks() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        let external_recording = temp.path().join("external.wav");
        std::fs::write(&external_recording, "wav").expect("write external recording");
        std::os::unix::fs::symlink(&external_recording, recordings_dir.join("linked.wav"))
            .expect("create recording symlink");

        let error = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "linked.wav")
            .expect_err("reject recording symlink");

        assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
    }

    #[cfg(unix)]
    fn symlinked_recordings_root_with_sentinel() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let external_recordings_dir = temp.path().join("external-recordings");
        std::fs::create_dir(&external_recordings_dir)
            .expect("create external recordings directory");
        let sentinel = external_recordings_dir.join("murmur-100.wav");
        std::fs::write(&sentinel, "sentinel").expect("write external sentinel");
        let recordings_dir = temp.path().join("recordings");
        std::os::unix::fs::symlink(&external_recordings_dir, &recordings_dir)
            .expect("symlink recordings directory");

        (temp, recordings_dir, sentinel)
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_recordings_root_for_playback_resolution() {
        let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();

        let error =
            HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "murmur-100.wav")
                .expect_err("reject symlinked recordings root");

        assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
        assert_eq!(
            std::fs::read_to_string(&sentinel).expect("read external sentinel"),
            "sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn deletion_with_symlinked_recordings_root_keeps_external_sentinel_and_removes_database_row() {
        let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();
        let conn = setup_conn();
        let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

        HistoryManager::delete_entry_with_conn(&conn, &recordings_dir, id)
            .expect("delete history entry with symlinked recordings root");

        assert_eq!(
            std::fs::read_to_string(&sentinel).expect("read external sentinel"),
            "sentinel"
        );
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count history entries");
        assert_eq!(remaining, 0);
    }

    #[cfg(unix)]
    #[test]
    fn retention_with_symlinked_recordings_root_keeps_external_sentinel_and_removes_database_row() {
        let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();
        let conn = setup_conn();
        let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

        let deleted = HistoryManager::delete_entries_and_files_with_conn(
            &conn,
            &recordings_dir,
            &[(id, "murmur-100.wav".to_string())],
        )
        .expect("retain history entry with symlinked recordings root");

        assert_eq!(deleted, 0);
        assert_eq!(
            std::fs::read_to_string(&sentinel).expect("read external sentinel"),
            "sentinel"
        );
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count history entries");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn rejects_missing_or_non_directory_recordings_roots() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let missing_root = temp.path().join("missing-recordings");
        let non_directory_root = temp.path().join("recordings-file");
        std::fs::write(&non_directory_root, "not a directory").expect("write recordings file");

        for recordings_dir in [&missing_root, &non_directory_root] {
            let error =
                HistoryManager::resolve_recording_path_in_dir(recordings_dir, "murmur-100.wav")
                    .expect_err("reject invalid recordings root");
            assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
        }
    }

    #[test]
    fn deletion_of_invalid_recording_keeps_external_sentinel_and_removes_database_row() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        let sentinel = temp.path().join("sentinel.wav");
        std::fs::write(&sentinel, "sentinel").expect("write sentinel");
        let conn = setup_conn();
        let id = insert_entry_with_file_name(&conn, "../sentinel.wav");

        HistoryManager::delete_entry_with_conn(&conn, &recordings_dir, id)
            .expect("delete invalid history entry");

        assert_eq!(
            std::fs::read_to_string(&sentinel).expect("read sentinel"),
            "sentinel"
        );
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count history entries");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn retention_of_invalid_recording_keeps_external_sentinel_and_removes_database_row() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        let sentinel = temp.path().join("sentinel.wav");
        std::fs::write(&sentinel, "sentinel").expect("write sentinel");
        let conn = setup_conn();
        let id = insert_entry_with_file_name(&conn, "../sentinel.wav");

        let deleted = HistoryManager::delete_entries_and_files_with_conn(
            &conn,
            &recordings_dir,
            &[(id, "../sentinel.wav".to_string())],
        )
        .expect("retain invalid history entry");

        assert_eq!(deleted, 0);
        assert_eq!(
            std::fs::read_to_string(&sentinel).expect("read sentinel"),
            "sentinel"
        );
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count history entries");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn retention_deletes_valid_recording_and_database_row() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let recordings_dir = temp.path().join("recordings");
        std::fs::create_dir(&recordings_dir).expect("create recordings directory");
        let recording = recordings_dir.join("murmur-100.wav");
        std::fs::write(&recording, "wav").expect("write recording");
        let conn = setup_conn();
        let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

        let deleted = HistoryManager::delete_entries_and_files_with_conn(
            &conn,
            &recordings_dir,
            &[(id, "murmur-100.wav".to_string())],
        )
        .expect("retain valid history entry");

        assert_eq!(deleted, 1);
        assert!(!recording.exists());
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count history entries");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn migration_from_version_four_preserves_history_and_removes_retired_columns() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );
            INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text,
                post_processed_text, post_process_prompt, post_process_requested
            ) VALUES (
                'murmur-100.wav', 100, 1, 'Recording 100', 'preserved',
                'retired output', 'retired prompt', 1
            );
            PRAGMA user_version = 4;",
        )
        .expect("create version-four history database");

        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("upgrade version-four database");

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read migrated version");
        assert_eq!(version, 5);

        let columns = conn
            .prepare("PRAGMA table_info(transcription_history)")
            .expect("prepare table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table info")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect column names");
        assert_eq!(
            columns,
            [
                "id",
                "file_name",
                "timestamp",
                "saved",
                "title",
                "transcription_text"
            ]
        );

        let preserved: (String, bool) = conn
            .query_row(
                "SELECT transcription_text, saved FROM transcription_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read preserved history entry");
        assert_eq!(preserved, ("preserved".to_string(), true));
    }

    #[test]
    fn fresh_migrations_create_only_the_cloud_transcription_schema() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("migrate fresh database");

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read migrated version");
        assert_eq!(version, 5);

        let columns = conn
            .prepare("PRAGMA table_info(transcription_history)")
            .expect("prepare table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table info")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect column names");
        assert_eq!(
            columns,
            [
                "id",
                "file_name",
                "timestamp",
                "saved",
                "title",
                "transcription_text"
            ]
        );
    }

    #[test]
    fn legacy_sql_plugin_tracking_is_converted_before_cloud_only_migration() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE _sqlx_migrations (
                version INTEGER PRIMARY KEY,
                success BOOLEAN NOT NULL
            );
            INSERT INTO _sqlx_migrations (version, success)
                VALUES (1, 1), (2, 1), (3, 1), (4, 1);
            CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );",
        )
        .expect("create legacy SQL-plugin database");

        migrate_from_tauri_plugin_sql(&conn).expect("convert legacy tracking");
        let converted_version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read converted version");
        assert_eq!(converted_version, 4);

        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("apply cloud-only migration");
        let final_version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read final version");
        assert_eq!(final_version, 5);
    }
}
