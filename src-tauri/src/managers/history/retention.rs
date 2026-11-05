use anyhow::Result;
use chrono::Utc;
use log::{debug, error};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tauri_specta::Event;

use super::{HistoryManager, HistoryUpdatePayload};

const RETENTION_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

impl HistoryManager {
    /// Applies the retention setting at launch and then every hour, so an
    /// age limit holds even when nothing new is dictated.
    pub fn start_retention_sweep(self: Arc<Self>) {
        let spawned = std::thread::Builder::new()
            .name("history-retention".into())
            .spawn(move || loop {
                if let Err(error) = self.cleanup_old_entries() {
                    error!("Scheduled history cleanup failed: {}", error);
                }
                std::thread::sleep(RETENTION_SWEEP_INTERVAL);
            });
        if let Err(error) = spawned {
            error!("Failed to start the history retention sweep: {}", error);
        }
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period = crate::settings::get_recording_retention_period(&self.app_handle);

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Keeps the newest `history_limit` unsaved entries.
                let limit = crate::settings::get_history_limit(&self.app_handle);
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    /// Deletes the unstarred entries among `entries` and their recordings, and
    /// returns the ids of the rows it deleted. `saved` is checked again here:
    /// the user may have starred an entry since the cleanup selected it.
    pub(super) fn delete_entries_and_files_with_conn(
        conn: &Connection,
        recordings_dir: &Path,
        entries: &[(i64, String)],
    ) -> Result<Vec<i64>> {
        let mut deleted = Vec::new();

        for (id, file_name) in entries {
            let rows = conn.execute(
                "DELETE FROM transcription_history WHERE id = ?1 AND saved = 0",
                params![id],
            )?;
            if rows == 0 {
                continue;
            }
            Self::delete_recording_file(recordings_dir, file_name);
            deleted.push(*id);
        }

        Ok(deleted)
    }

    /// Deletes `entries` like `delete_entries_and_files_with_conn` and tells
    /// the History page which rows went away.
    fn delete_entries_and_files(&self, entries: &[(i64, String)]) -> Result<usize> {
        let conn = self.get_connection()?;
        let deleted =
            Self::delete_entries_and_files_with_conn(&conn, &self.recordings_dir, entries)?;
        for &id in &deleted {
            if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
                error!("Failed to emit history-updated event: {}", e);
            }
        }
        Ok(deleted.len())
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

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

        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60),
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60),
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60),
            _ => unreachable!("Should not reach here"),
        };

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
}
