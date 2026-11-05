use anyhow::Result;
use rusqlite::params;

use super::types::{AudioState, MeetingStatus, SummaryStatus};
use super::MeetingsStore;
use crate::meetings::paths;
use crate::meetings::retention::MeetingAudio;

impl MeetingsStore {
    /// One more attempt to compress the audio of meeting `id` failed.
    pub fn compress_failed(&self, id: i64) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET compress_failures = compress_failures + 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// The tracks are compressed into `audio.m4a`. Returns false when the
    /// user deleted the audio meanwhile.
    pub fn audio_compressed(&self, id: i64, bytes: u64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET audio_file = ?2, audio_bytes = ?3
             WHERE id = ?1 AND audio_deleted = 0",
            params![id, paths::AUDIO_FILE, bytes as i64],
        )?;
        Ok(updated > 0)
    }

    /// Every meeting that still has audio, as retention weighs it.
    pub fn with_audio(&self) -> Result<Vec<MeetingAudio>> {
        let conn = self.connection()?;
        let rows = conn
            .prepare(
                "SELECT id, started_at_ms, status, summary_status, audio_file
                 FROM meetings WHERE audio_deleted = 0",
            )?
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, started_at_ms, status, summary, audio_file)| {
                Some(MeetingAudio {
                    id,
                    started_at_ms,
                    status: MeetingStatus::from_db(&status)?,
                    summary: SummaryStatus::from_db(&summary),
                    audio: match audio_file {
                        Some(_) => AudioState::Compressed,
                        None => AudioState::Tracks,
                    },
                })
            })
            .collect())
    }

    /// Deletes a finished meeting's audio; its text stays. The guarded
    /// update reserves the SQLite writer before removing files, so a
    /// concurrent retranscribe/reopen cannot start using them. Only commit
    /// after removal succeeds: an error or interruption keeps deletion
    /// retryable, including when some files have already gone.
    /// `remove_files` must only touch the filesystem, not write to the store.
    /// A compression finishing meanwhile waits for the commit, then finds the
    /// audio deleted. Returns false while the meeting records or processes.
    pub fn delete_audio(
        &self,
        id: i64,
        remove_files: impl FnOnce() -> std::io::Result<()>,
    ) -> Result<bool> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE meetings SET audio_deleted = 1, audio_file = NULL, audio_bytes = 0
             WHERE id = ?1 AND status IN ('ready', 'failed')",
            params![id],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        remove_files()?;
        tx.commit()?;
        Ok(true)
    }
}
