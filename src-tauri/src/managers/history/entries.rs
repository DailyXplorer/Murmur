use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use tauri_specta::Event;

use super::{HistoryEntry, HistoryHit, HistoryManager, HistoryUpdatePayload, PaginatedHistory};
use crate::cleanup::{CleanupOutcome, ProcessedText};
use crate::engine::{Engine, EngineUsed, FallbackReason};
use crate::search::{fts_query, Snippet};

/// The columns `map_history_entry` reads.
const ENTRY_COLUMNS: &str =
    "id, file_name, timestamp, saved, title, transcription_text, raw_text, cleanup, engine, fallback_reason";

/// The raw transcript is kept only when cleanup changed the delivered text;
/// a failed cleanup delivered the raw text itself.
pub(super) fn cleanup_columns(text: &ProcessedText) -> (Option<&str>, Option<CleanupOutcome>) {
    let raw = (text.cleanup == Some(CleanupOutcome::Cleaned)).then_some(text.raw.as_str());
    (raw, text.cleanup)
}

/// Inserts an unsaved row and returns its id.
pub(super) fn insert_row(
    conn: &Connection,
    file_name: &str,
    timestamp: i64,
    title: &str,
    text: &ProcessedText,
    source: EngineUsed,
) -> Result<i64> {
    let (raw_text, cleanup) = cleanup_columns(text);
    conn.execute(
        "INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text, raw_text, cleanup,
            engine, fallback_reason
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            file_name,
            timestamp,
            false,
            title,
            &text.text,
            raw_text,
            cleanup.map(CleanupOutcome::as_str),
            source.engine.as_str(),
            source.fallback.map(FallbackReason::as_str),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

impl HistoryManager {
    /// Reads the entries `clause` selects. Every read of whole entries goes
    /// through here, so the columns always match `map_history_entry`.
    fn query_entries(
        conn: &Connection,
        clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<HistoryEntry>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM transcription_history {clause}"
        ))?;
        let entries = stmt
            .query_map(params, Self::map_history_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    fn query_entry(
        conn: &Connection,
        clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<HistoryEntry>> {
        Ok(Self::query_entries(conn, clause, params)?
            .into_iter()
            .next())
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            raw_text: row.get("raw_text")?,
            cleanup: row
                .get::<_, Option<String>>("cleanup")?
                .as_deref()
                .and_then(CleanupOutcome::parse),
            engine: Engine::parse(&row.get::<_, String>("engine")?).unwrap_or(Engine::Cloud),
            fallback_reason: row
                .get::<_, Option<String>>("fallback_reason")?
                .as_deref()
                .and_then(FallbackReason::parse),
        })
    }

    /// Save a new history entry to the database.
    /// The WAV file should already have been written to the recordings directory.
    pub fn save_entry(
        &self,
        file_name: String,
        text: ProcessedText,
        source: EngineUsed,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        let id = insert_row(&conn, &file_name, timestamp, &title, &text, source)?;
        let (raw_text, cleanup) = cleanup_columns(&text);
        let raw_text = raw_text.map(str::to_string);

        let entry = HistoryEntry {
            id,
            file_name,
            timestamp,
            saved: false,
            title,
            transcription_text: text.text,
            raw_text,
            cleanup,
            engine: source.engine,
            fallback_reason: source.fallback,
        };

        debug!("Saved history entry with id {}", entry.id);

        // The row has committed at this point, so a cleanup failure must not
        // report the save as failed. Callers use success to transfer WAV
        // ownership and deleting that referenced file would corrupt history.
        if let Err(error) = self.cleanup_old_entries() {
            error!(
                "History cleanup failed after saving entry {}: {}",
                entry.id, error
            );
        }

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
        text: ProcessedText,
        source: EngineUsed,
    ) -> Result<HistoryEntry> {
        let (raw_text, cleanup) = cleanup_columns(&text);
        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE transcription_history
                SET transcription_text = ?1, raw_text = ?2, cleanup = ?3,
                    engine = ?4, fallback_reason = ?5
              WHERE id = ?6",
            params![
                text.text,
                raw_text,
                cleanup.map(CleanupOutcome::as_str),
                source.engine.as_str(),
                source.fallback.map(FallbackReason::as_str),
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = Self::query_entry(&conn, "WHERE id = ?1", params![id])?
            .ok_or_else(|| anyhow!("History entry {} not found", id))?;

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

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => Self::query_entries(
                &conn,
                "WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
                params![cursor_id, (lim + 1) as i64],
            )?,
            (None, Some(lim)) => Self::query_entries(
                &conn,
                "ORDER BY id DESC LIMIT ?1",
                params![(lim + 1) as i64],
            )?,
            (_, None) => Self::query_entries(&conn, "ORDER BY id DESC", [])?,
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<HistoryHit>> {
        Self::search_with_conn(&self.get_connection()?, query, limit)
    }

    /// The dictations whose delivered text matches `query`, most relevant
    /// first, then newest. Read-only: it works on any connection to
    /// `history.db`.
    pub fn search_with_conn(
        conn: &Connection,
        query: &str,
        limit: usize,
    ) -> Result<Vec<HistoryHit>> {
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut statement = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS}, marked FROM transcription_history
             JOIN (SELECT rowid AS hit_id, bm25(history_fts) AS score,
                          highlight(history_fts, 0, char(1), char(2)) AS marked
                   FROM history_fts WHERE history_fts MATCH ?1)
               ON id = hit_id
             ORDER BY score, timestamp DESC LIMIT ?2"
        ))?;
        let hits = statement
            .query_map(params![fts, limit as i64], |row| {
                Ok(HistoryHit {
                    entry: Self::map_history_entry(row)?,
                    ranges: Snippet::from_marked(&row.get::<_, String>("marked")?).ranges,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hits)
    }

    #[cfg(test)]
    pub(super) fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        Self::query_entry(conn, "ORDER BY timestamp DESC LIMIT 1", [])
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    pub(super) fn get_latest_completed_entry_with_conn(
        conn: &Connection,
    ) -> Result<Option<HistoryEntry>> {
        Self::query_entry(
            conn,
            "WHERE transcription_text != '' ORDER BY timestamp DESC LIMIT 1",
            [],
        )
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

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
        Self::get_entry_by_id_with_conn(&conn, id)
    }

    pub(super) fn get_entry_by_id_with_conn(
        conn: &Connection,
        id: i64,
    ) -> Result<Option<HistoryEntry>> {
        Self::query_entry(conn, "WHERE id = ?1", [id])
    }

    pub(super) fn delete_entry_with_conn(
        conn: &Connection,
        recordings_dir: &Path,
        id: i64,
    ) -> Result<()> {
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

        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }
}
