use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

use super::types::{
    AudioState, MeetingDetail, MeetingListItem, MeetingSpeaker, MeetingStage, MeetingStatus,
    MeetingTracks, MeetingTurn, MeetingsPage, ProcessingError, SpeakerAnalysis, SummaryStatus,
};
use super::MeetingsStore;
use crate::meetings::capture::route::OutputRoute;
use crate::meetings::summary::SummaryError;

/// Newest first; `cursor` is the id of the last meeting already shown.
/// Read-only: it works on any connection to `meetings.db`.
pub fn list_page(conn: &Connection, cursor: Option<i64>, limit: usize) -> Result<MeetingsPage> {
    let mut statement = conn.prepare(
        "SELECT id, title, started_at_ms, duration_ms, status, capture_mode
         FROM meetings WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let mut meetings = statement
        .query_map(
            params![cursor.unwrap_or(i64::MAX), limit as i64 + 1],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )?
        .filter_map(|row| match row {
            Ok((id, title, started_at_ms, duration_ms, status, tracks)) => {
                match (
                    MeetingStatus::from_db(&status),
                    MeetingTracks::from_db(&tracks),
                ) {
                    (Some(status), Some(tracks)) => Some(Ok(MeetingListItem {
                        id,
                        title,
                        started_at_ms,
                        duration_ms,
                        status,
                        tracks,
                    })),
                    _ => None,
                }
            }
            Err(error) => Some(Err(error)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = meetings.len() > limit;
    meetings.truncate(limit);
    Ok(MeetingsPage { meetings, has_more })
}

impl MeetingsStore {
    pub fn insert_recording(&self, started_at_ms: i64, tracks: MeetingTracks) -> Result<i64> {
        self.insert_meeting(started_at_ms, tracks, None)
    }

    /// Like `insert_recording`, for audio that already exists: `title` is the
    /// meeting's name until its summary gives it a better one (the user's
    /// own rename always wins).
    pub fn insert_import(
        &self,
        started_at_ms: i64,
        tracks: MeetingTracks,
        title: Option<&str>,
    ) -> Result<i64> {
        self.insert_meeting(started_at_ms, tracks, title)
    }

    fn insert_meeting(
        &self,
        started_at_ms: i64,
        tracks: MeetingTracks,
        title: Option<&str>,
    ) -> Result<i64> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE meeting_last_id
             SET id = MAX(id, (SELECT COALESCE(MAX(id), 0) FROM meetings)) + 1",
            [],
        )?;
        tx.execute(
            "INSERT INTO meetings (id, started_at_ms, capture_mode, status, title)
             VALUES ((SELECT id FROM meeting_last_id), ?1, ?2, 'recording', ?3)",
            params![started_at_ms, tracks.as_db(), title],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub fn set_tracks(&self, id: i64, tracks: MeetingTracks) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET capture_mode = ?2 WHERE id = ?1",
            params![id, tracks.as_db()],
        )?;
        Ok(())
    }

    /// The recording is complete and waits for processing.
    pub fn finish_recording(&self, id: i64, ended_at_ms: i64, duration_ms: i64) -> Result<()> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET ended_at_ms = ?2, duration_ms = ?3, status = 'processing'
             WHERE id = ?1 AND status = 'recording'",
            params![id, ended_at_ms, duration_ms],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {id} is not recording"));
        }
        Ok(())
    }

    /// Removes the meeting's rows; its chunks, speakers and turns cascade.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.connection()?
            .execute("DELETE FROM meetings WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// The id of every meeting in the library.
    pub fn ids(&self) -> Result<HashSet<i64>> {
        let conn = self.connection()?;
        let ids = conn
            .prepare("SELECT id FROM meetings")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<i64>>>()?;
        Ok(ids)
    }

    /// Newest first; `cursor` is the id of the last meeting already shown.
    pub fn list(&self, cursor: Option<i64>, limit: usize) -> Result<MeetingsPage> {
        list_page(&self.connection()?, cursor, limit)
    }

    /// Notes how much audio the recording of meeting `id` dropped.
    pub fn set_dropped_ms(&self, id: i64, dropped_ms: u64) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET dropped_ms = ?2 WHERE id = ?1",
            params![id, i64::try_from(dropped_ms).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn dropped_ms(&self, id: i64) -> Result<i64> {
        Ok(self.connection()?.query_row(
            "SELECT dropped_ms FROM meetings WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?)
    }

    /// Notes that the Mac slept `slept_ms` more while meeting `id` recorded.
    pub fn add_slept_ms(&self, id: i64, slept_ms: u64) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET slept_ms = slept_ms + ?2 WHERE id = ?1",
            params![id, i64::try_from(slept_ms).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn slept_ms(&self, id: i64) -> Result<i64> {
        Ok(self.connection()?.query_row(
            "SELECT slept_ms FROM meetings WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?)
    }

    pub fn get(&self, id: i64) -> Result<Option<MeetingDetail>> {
        let conn = self.connection()?;
        let row = conn
            .query_row(
                "SELECT title, started_at_ms, duration_ms, status, capture_mode, stage, error_kind,
                        speaker_analysis, summary_md, summary_status, summary_error,
                        audio_file, audio_bytes, audio_deleted, system_audio_silent
                 FROM meetings WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        (
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ),
                        (
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<String>>(11)?,
                            row.get::<_, i64>(12)?,
                            row.get::<_, bool>(13)?,
                            row.get::<_, bool>(14)?,
                        ),
                    ))
                },
            )
            .optional()?;
        let Some((
            (title, started_at_ms, duration_ms, status, tracks, stage, error, analysis),
            (
                summary,
                summary_status,
                summary_error,
                audio_file,
                audio_bytes,
                audio_deleted,
                system_audio_silent,
            ),
        )) = row
        else {
            return Ok(None);
        };
        let speakers = conn
            .prepare(
                "SELECT key, display_name, default_name, color_index FROM meeting_speakers
                 WHERE meeting_id = ?1 ORDER BY color_index, key",
            )?
            .query_map(params![id], |row| {
                Ok(MeetingSpeaker {
                    key: row.get(0)?,
                    name: row.get(1)?,
                    default_name: row.get(2)?,
                    color_index: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let turns = conn
            .prepare(
                "SELECT speaker_key, start_ms, end_ms, text, low_confidence FROM meeting_turns
                 WHERE meeting_id = ?1 ORDER BY seq",
            )?
            .query_map(params![id], |row| {
                Ok(MeetingTurn {
                    speaker_key: row.get(0)?,
                    start_ms: row.get(1)?,
                    end_ms: row.get(2)?,
                    text: row.get(3)?,
                    low_confidence: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let local_fallback_ms = conn.query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM meeting_chunks
             WHERE meeting_id = ?1 AND status = 'done' AND error_kind IS NOT NULL",
            params![id],
            |row| row.get(0),
        )?;
        Ok(Some(MeetingDetail {
            id,
            title,
            started_at_ms,
            duration_ms,
            status: MeetingStatus::from_db(&status)
                .ok_or_else(|| anyhow!("Meeting {id} has an unknown status"))?,
            tracks: MeetingTracks::from_db(&tracks).unwrap_or(MeetingTracks::Both),
            stage: stage.as_deref().and_then(MeetingStage::from_db),
            error: error.as_deref().and_then(ProcessingError::from_db),
            speaker_analysis: analysis.as_deref().and_then(SpeakerAnalysis::from_db),
            speakers,
            turns,
            summary,
            summary_status: SummaryStatus::from_db(&summary_status),
            summary_error: summary_error.as_deref().and_then(SummaryError::from_db),
            audio: match (audio_deleted, audio_file) {
                (true, _) => AudioState::Deleted,
                (false, Some(_)) => AudioState::Compressed,
                (false, None) => AudioState::Tracks,
            },
            audio_bytes,
            system_audio_silent,
            held_until_ms: None,
            waiting_for_connection: false,
            local_fallback_ms,
        }))
    }

    pub fn set_system_audio_silent(&self, id: i64, silent: bool) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET system_audio_silent = ?2 WHERE id = ?1 AND status = 'processing'",
            params![id, silent],
        )?;
        Ok(())
    }

    /// Notes that another app played sound while meeting `id` recorded
    /// system audio, so system audio of pure zeros means it was withheld.
    pub fn set_system_played(&self, id: i64) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET system_played = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn system_played(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT system_played FROM meetings WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Notes where meeting `id` played sound while it recorded.
    pub fn set_output_route(&self, id: i64, route: OutputRoute) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meetings SET output_route = ?2 WHERE id = ?1",
            params![id, route.as_str()],
        )?;
        Ok(())
    }

    pub fn output_route(&self, id: i64) -> Result<OutputRoute> {
        let name: Option<String> = self
            .connection()?
            .query_row(
                "SELECT output_route FROM meetings WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(OutputRoute::parse(name.as_deref()))
    }

    /// `None` restores the default name.
    pub fn rename(&self, id: i64, title: Option<&str>) -> Result<()> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET title = ?2, title_edited = ?3 WHERE id = ?1",
            params![id, title, title.is_some()],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {id} does not exist"));
        }
        Ok(())
    }

    pub fn rename_speaker(&self, id: i64, key: &str, name: &str) -> Result<()> {
        let updated = self.connection()?.execute(
            "UPDATE meeting_speakers SET display_name = ?3 WHERE meeting_id = ?1 AND key = ?2",
            params![id, key, name],
        )?;
        if updated == 0 {
            return Err(anyhow!("Meeting {id} has no speaker {key}"));
        }
        Ok(())
    }
}
