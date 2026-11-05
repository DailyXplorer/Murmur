use anyhow::Result;
use log::warn;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

use super::types::{MeetingStage, MeetingStatus, MeetingTracks, ProcessingError};
use super::MeetingsStore;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::meetings::paths;
use crate::meetings::wav_recovery::repair_wav;

/// Attempts at compressing a meeting's audio before its tracks are kept as
/// they are: a file that fails this often fails for good.
const MAX_COMPRESS_FAILURES: i64 = 3;

/// A meeting a crash cut off with less audio than this has nothing to
/// transcribe.
const MIN_RECOVERED_MS: u64 = 1_000;

impl MeetingsStore {
    /// At startup no meeting can still be recording: one that is was cut off
    /// by a crash or a forced quit. Its track headers are repaired so the
    /// audio is readable, and it is marked interrupted with the duration the
    /// tracks hold, to be processed like any other; with less than a second
    /// of audio there is nothing to process, and it fails with `NoAudio`.
    /// Returns the meetings left to process.
    pub fn recover_interrupted(&self, root: &Path) -> Result<Vec<i64>> {
        let conn = self.connection()?;
        let cut_off = conn
            .prepare("SELECT id, capture_mode FROM meetings WHERE status = 'recording'")?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut recovered = Vec::with_capacity(cut_off.len());
        for (id, tracks) in &cut_off {
            let dir = paths::meeting_dir(root, *id);
            let tracks = MeetingTracks::from_db(tracks).unwrap_or(MeetingTracks::Both);
            let samples = [
                (tracks.has_mic(), paths::MIC_TRACK),
                (tracks.has_system(), paths::SYSTEM_TRACK),
            ]
            .into_iter()
            .filter(|(recorded, _)| *recorded)
            .filter_map(|(_, file)| {
                repair_wav(&dir.join(file))
                    .inspect_err(|error| warn!("Meeting {id}: cannot repair {file}: {error}"))
                    .ok()
            })
            .max()
            .unwrap_or(0);
            let duration_ms = samples * 1000 / u64::from(TARGET_SAMPLE_RATE);
            if duration_ms < MIN_RECOVERED_MS {
                conn.execute(
                    "UPDATE meetings SET status = 'failed', error_kind = ?3, duration_ms = ?2
                     WHERE id = ?1",
                    params![id, duration_ms as i64, ProcessingError::NoAudio.as_db()],
                )?;
                continue;
            }
            conn.execute(
                "UPDATE meetings SET status = 'interrupted', duration_ms = ?2 WHERE id = ?1",
                params![id, duration_ms as i64],
            )?;
            recovered.push(*id);
        }
        Ok(recovered)
    }

    /// Meetings to process, oldest first: those a quit left mid-way, those a
    /// crash cut off, those that waited for the connection, and transcribed
    /// ones still waiting for their summary or compressed audio. Audio that
    /// failed to compress `MAX_COMPRESS_FAILURES` times keeps its tracks
    /// instead of being tried at every launch.
    pub fn waiting_for_processing(&self) -> Result<Vec<i64>> {
        Ok(self
            .connection()?
            .prepare(
                "SELECT id FROM meetings
                 WHERE status IN ('processing', 'interrupted')
                    OR (status = 'failed' AND error_kind = 'offline')
                    OR (status = 'ready' AND (summary_status = 'pending'
                        OR (audio_file IS NULL AND audio_deleted = 0
                            AND compress_failures < ?1)))
                 ORDER BY id",
            )?
            .query_map(params![MAX_COMPRESS_FAILURES], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Makes a finished meeting with audio wait to be transcribed from
    /// scratch: its pieces are forgotten. Its transcript and summary stay
    /// until a new transcript replaces them, as the audio may turn out
    /// unreadable or Codex may not answer. Returns false when it is not
    /// finished or has no audio.
    pub fn retranscribe(&self, id: i64) -> Result<bool> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE meetings SET status = 'processing', stage = NULL, error_kind = NULL
             WHERE id = ?1 AND status IN ('ready', 'failed') AND audio_deleted = 0",
            params![id],
        )?;
        if updated > 0 {
            tx.execute(
                "DELETE FROM meeting_chunks WHERE meeting_id = ?1",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(updated > 0)
    }

    /// Starts (or resumes) processing a recorded meeting. Returns false when
    /// the meeting is gone, still recording, or already done.
    pub fn begin_processing(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET status = 'processing', stage = 'transcribing',
                 error_kind = NULL, error_detail = NULL
             WHERE id = ?1 AND status IN ('processing', 'interrupted', 'failed')",
            params![id],
        )?;
        Ok(updated > 0)
    }

    /// Makes a failed or interrupted meeting wait for processing, as Retry
    /// asks: it shows as waiting while other meetings go first, not as still
    /// failed. `begin_processing` clears its error when its turn comes.
    /// Returns false when it is neither.
    pub fn wait_for_retry(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET status = 'processing', stage = NULL
             WHERE id = ?1 AND status IN ('failed', 'interrupted')",
            params![id],
        )?;
        Ok(updated > 0)
    }

    /// Makes a finished meeting wait for processing again, to tell its
    /// speakers apart anew. Returns false when it is not finished or has no
    /// audio.
    pub fn reopen(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET status = 'processing', stage = NULL
             WHERE id = ?1 AND status IN ('ready', 'failed') AND audio_deleted = 0",
            params![id],
        )?;
        Ok(updated > 0)
    }

    pub fn set_stage(&self, id: i64, stage: MeetingStage) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET stage = ?2 WHERE id = ?1 AND status = 'processing'",
            params![id, stage.as_db()],
        )?;
        Ok(())
    }

    /// Ends a processing run: ready, or failed with what went wrong.
    pub fn finish_processing(&self, id: i64, error: Option<ProcessingError>) -> Result<bool> {
        let status = if error.is_some() {
            MeetingStatus::Failed
        } else {
            MeetingStatus::Ready
        };
        let changed = self.connection()?.execute(
            "UPDATE meetings SET status = ?2, stage = NULL, error_kind = ?3
             WHERE id = ?1 AND status = 'processing'",
            params![id, status.as_db(), error.map(ProcessingError::as_db)],
        )?;
        Ok(changed > 0)
    }

    pub fn announcement_name(
        &self,
        id: i64,
    ) -> Result<Option<crate::meetings::announce::MeetingName>> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT title, started_at_ms FROM meetings WHERE id = ?1",
                [id],
                |row| {
                    Ok(crate::meetings::announce::MeetingName {
                        title: row.get(0)?,
                        started_at_ms: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }
}
