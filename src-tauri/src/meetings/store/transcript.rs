use anyhow::Result;
use rusqlite::params;

use super::types::{
    track_db, track_from_db, ChunkRow, ChunkStatus, MeetingSpeaker, MeetingTurn, SpeakerAnalysis,
};
use super::MeetingsStore;
use crate::meetings::chunking::ChunkPlan;
use crate::meetings::merge::Track;

impl MeetingsStore {
    /// Records how a track is split, once: a plan already stored is kept, so
    /// pieces already transcribed are never sent again. Only a meeting being
    /// processed gets one: a deleted meeting's id goes to the next meeting.
    pub fn plan_chunks(&self, id: i64, track: Track, plan: &[ChunkPlan]) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let unplanned: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM meetings WHERE id = ?1 AND status = 'processing')
                 AND NOT EXISTS (SELECT 1 FROM meeting_chunks WHERE meeting_id = ?1 AND track = ?2)",
            params![id, track_db(track)],
            |row| row.get(0),
        )?;
        if unplanned {
            for (idx, chunk) in plan.iter().enumerate() {
                tx.execute(
                    "INSERT INTO meeting_chunks (meeting_id, track, idx, start_ms, end_ms, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        track_db(track),
                        idx as i64,
                        chunk.start_ms as i64,
                        chunk.end_ms as i64,
                        if chunk.silent() { "silent" } else { "pending" }
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn chunks(&self, id: i64) -> Result<Vec<ChunkRow>> {
        let conn = self.connection()?;
        let rows = conn
            .prepare(
                "SELECT track, idx, start_ms, end_ms, status, text FROM meeting_chunks
                 WHERE meeting_id = ?1 ORDER BY start_ms, track DESC, idx",
            )?
            .query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(track, idx, start_ms, end_ms, status, text)| {
                Some(ChunkRow {
                    track: track_from_db(&track)?,
                    idx: idx as u32,
                    start_ms: start_ms as u64,
                    end_ms: end_ms as u64,
                    status: ChunkStatus::from_db(&status),
                    text,
                })
            })
            .collect())
    }

    pub fn chunk_done(&self, id: i64, track: Track, idx: u32, text: &str) -> Result<()> {
        self.save_chunk(id, track, idx, text, None)
    }

    /// A piece Codex refused with `codex`, transcribed on this Mac. A done
    /// piece keeps the Codex failure it got past, which marks it as local.
    pub fn chunk_done_locally(
        &self,
        id: i64,
        track: Track,
        idx: u32,
        text: &str,
        codex: &str,
    ) -> Result<()> {
        self.save_chunk(id, track, idx, text, Some(codex))
    }

    fn save_chunk(
        &self,
        id: i64,
        track: Track,
        idx: u32,
        text: &str,
        codex: Option<&str>,
    ) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meeting_chunks SET status = 'done', text = ?4, error_kind = ?5,
                 attempts = attempts + 1
             WHERE meeting_id = ?1 AND track = ?2 AND idx = ?3",
            params![id, track_db(track), idx, text, codex],
        )?;
        Ok(())
    }

    pub fn chunk_failed(&self, id: i64, track: Track, idx: u32, kind: &str) -> Result<()> {
        self.connection()?.execute(
            "UPDATE meeting_chunks SET status = 'failed', error_kind = ?4,
                 attempts = attempts + 1
             WHERE meeting_id = ?1 AND track = ?2 AND idx = ?3",
            params![id, track_db(track), idx, kind],
        )?;
        Ok(())
    }

    /// Replaces the transcript and its speakers in one transaction, with how
    /// the speakers were told apart. A speaker still present keeps the name
    /// the user gave them, unless `identity` (the diarization the keys come
    /// from, `None` for one speaker per track) changed: the same key may
    /// then be someone else, so remote speakers get their default names.
    /// Only a meeting being processed gets one, as with `plan_chunks`.
    pub fn save_transcript(
        &self,
        id: i64,
        speakers: &[MeetingSpeaker],
        turns: &[MeetingTurn],
        analysis: SpeakerAnalysis,
        identity: Option<&str>,
        alignment_score: Option<f32>,
    ) -> Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let processing: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM meetings WHERE id = ?1 AND status = 'processing')",
            params![id],
            |row| row.get(0),
        )?;
        if !processing {
            return Ok(());
        }
        let previous: Vec<(String, String)> = tx
            .prepare(
                "SELECT speaker_key, text FROM meeting_turns WHERE meeting_id = ?1 ORDER BY seq",
            )?
            .query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let reworded = !previous
            .iter()
            .map(|(key, text)| (key.as_str(), text.as_str()))
            .eq(turns
                .iter()
                .map(|turn| (turn.speaker_key.as_str(), turn.text.as_str())));
        tx.execute(
            "DELETE FROM meeting_turns WHERE meeting_id = ?1",
            params![id],
        )?;
        tx.execute(
            "UPDATE meeting_speakers SET display_name = default_name
             WHERE meeting_id = ?1 AND key <> 'me'
               AND (SELECT speaker_identity FROM meetings WHERE id = ?1) IS NOT ?2",
            params![id, identity],
        )?;
        // The summary names people by key: with new keys or other words, it
        // is written anew.
        tx.execute(
            "UPDATE meetings SET summary_md = NULL, summary_status = 'pending',
                 summary_error = NULL
             WHERE id = ?1 AND (?3 OR speaker_identity IS NOT ?2)",
            params![id, identity, reworded],
        )?;
        tx.execute(
            "UPDATE meetings SET speaker_identity = ?2 WHERE id = ?1",
            params![id, identity],
        )?;
        let kept: Vec<String> = speakers.iter().map(|speaker| speaker.key.clone()).collect();
        tx.execute(
            "DELETE FROM meeting_speakers
             WHERE meeting_id = ?1 AND key NOT IN (SELECT value FROM json_each(?2))",
            params![id, serde_json::to_string(&kept)?],
        )?;
        tx.execute(
            "UPDATE meetings SET speaker_analysis = ?2, alignment_score = ?3 WHERE id = ?1",
            params![id, analysis.as_db(), alignment_score.map(f64::from)],
        )?;
        for (seq, turn) in turns.iter().enumerate() {
            tx.execute(
                "INSERT INTO meeting_turns
                     (meeting_id, seq, speaker_key, start_ms, end_ms, text, low_confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    seq as i64,
                    turn.speaker_key,
                    turn.start_ms,
                    turn.end_ms,
                    turn.text,
                    turn.low_confidence
                ],
            )?;
        }
        for speaker in speakers {
            tx.execute(
                "INSERT INTO meeting_speakers
                     (meeting_id, key, default_name, display_name, color_index)
                 VALUES (?1, ?2, ?3, ?3, ?4)
                 ON CONFLICT (meeting_id, key) DO UPDATE SET
                     display_name = CASE WHEN display_name = default_name
                         THEN excluded.default_name ELSE display_name END,
                     default_name = excluded.default_name,
                     color_index = excluded.color_index",
                params![id, speaker.key, speaker.default_name, speaker.color_index],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
