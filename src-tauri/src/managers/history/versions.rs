//! The transcripts a re-transcription replaced. A dictation keeps its five
//! most recent earlier transcripts so the user can compare them with the new
//! one and go back. They live in `transcription_versions`, outside the search
//! index, which only ever sees the current text.

use anyhow::{anyhow, Result};
use log::debug;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{HistoryEntry, HistoryManager};
use crate::cleanup::{CleanupOutcome, ProcessedText};
use crate::engine::{Engine, EngineUsed, FallbackReason};

/// How many earlier transcripts a dictation keeps.
pub const MAX_VERSIONS: usize = 5;

/// An earlier transcript of a dictation.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryVersion {
    pub id: i64,
    /// When this transcript was produced, in Unix seconds.
    pub produced_at: i64,
    pub transcription_text: String,
    /// The transcript before AI cleanup, when cleanup ran.
    pub raw_text: Option<String>,
    pub cleanup: Option<CleanupOutcome>,
    pub engine: Engine,
    pub fallback_reason: Option<FallbackReason>,
}

/// The fields a dictation and a version have in common, as stored: the engine
/// and reason stay raw strings so a value a newer build wrote survives a swap.
struct Transcript {
    text: String,
    raw_text: Option<String>,
    cleanup: Option<String>,
    engine: String,
    fallback_reason: Option<String>,
    produced_at: i64,
}

fn current_transcript(conn: &Connection, id: i64) -> Result<Transcript> {
    conn.query_row(
        "SELECT transcription_text, raw_text, cleanup, engine, fallback_reason,
                COALESCE(transcribed_at, timestamp)
           FROM transcription_history WHERE id = ?1",
        params![id],
        |row| {
            Ok(Transcript {
                text: row.get(0)?,
                raw_text: row.get(1)?,
                cleanup: row.get(2)?,
                engine: row.get(3)?,
                fallback_reason: row.get(4)?,
                produced_at: row.get(5)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("History entry {} not found", id))
}

fn write_current(conn: &Connection, id: i64, transcript: &Transcript) -> Result<()> {
    conn.execute(
        "UPDATE transcription_history
            SET transcription_text = ?1, raw_text = ?2, cleanup = ?3,
                engine = ?4, fallback_reason = ?5, transcribed_at = ?6
          WHERE id = ?7",
        params![
            transcript.text,
            transcript.raw_text,
            transcript.cleanup,
            transcript.engine,
            transcript.fallback_reason,
            transcript.produced_at,
            id
        ],
    )?;
    Ok(())
}

impl HistoryManager {
    /// The earlier transcripts of dictation `entry_id`, newest first.
    pub fn versions(&self, entry_id: i64) -> Result<Vec<HistoryVersion>> {
        Self::versions_with_conn(&self.get_connection()?, entry_id)
    }

    pub(super) fn versions_with_conn(
        conn: &Connection,
        entry_id: i64,
    ) -> Result<Vec<HistoryVersion>> {
        let mut statement = conn.prepare(
            "SELECT id, produced_at, transcription_text, raw_text, cleanup, engine,
                    fallback_reason
               FROM transcription_versions WHERE entry_id = ?1
              ORDER BY produced_at DESC, id DESC",
        )?;
        let versions = statement
            .query_map(params![entry_id], |row| {
                Ok(HistoryVersion {
                    id: row.get(0)?,
                    produced_at: row.get(1)?,
                    transcription_text: row.get(2)?,
                    raw_text: row.get(3)?,
                    cleanup: row
                        .get::<_, Option<String>>(4)?
                        .as_deref()
                        .and_then(CleanupOutcome::parse),
                    engine: Engine::parse(&row.get::<_, String>(5)?).unwrap_or(Engine::Cloud),
                    fallback_reason: row
                        .get::<_, Option<String>>(6)?
                        .as_deref()
                        .and_then(FallbackReason::parse),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(versions)
    }

    /// Stores `text` as the transcript of dictation `id`. The transcript it
    /// replaces becomes a version, unless it was empty (a failed first
    /// attempt) or is the same text, and only the newest `MAX_VERSIONS` stay.
    /// One transaction: a failure leaves the dictation as it was.
    pub(super) fn replace_transcription_with_conn(
        conn: &mut Connection,
        id: i64,
        text: &ProcessedText,
        source: EngineUsed,
        now: i64,
    ) -> Result<()> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = current_transcript(&tx, id)?;
        if !previous.text.trim().is_empty() && previous.text != text.text {
            tx.execute(
                "INSERT INTO transcription_versions (
                    entry_id, produced_at, transcription_text, raw_text, cleanup, engine,
                    fallback_reason
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    previous.produced_at,
                    previous.text,
                    previous.raw_text,
                    previous.cleanup,
                    previous.engine,
                    previous.fallback_reason
                ],
            )?;
            let added = tx.last_insert_rowid();
            // The text just replaced always stays: after a restore it can be
            // older than the versions kept, and the user was reading it.
            tx.execute(
                "DELETE FROM transcription_versions
                  WHERE entry_id = ?1 AND id NOT IN (
                      SELECT id FROM transcription_versions WHERE entry_id = ?1
                       ORDER BY id = ?3 DESC, produced_at DESC, id DESC LIMIT ?2)",
                params![id, MAX_VERSIONS as i64, added],
            )?;
        }
        let (raw_text, cleanup) = super::entries::cleanup_columns(text);
        write_current(
            &tx,
            id,
            &Transcript {
                text: text.text.clone(),
                raw_text: raw_text.map(str::to_string),
                cleanup: cleanup.map(|outcome| outcome.as_str().to_string()),
                engine: source.engine.as_str().to_string(),
                fallback_reason: source.fallback.map(|reason| reason.as_str().to_string()),
                produced_at: now,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Makes version `version_id` the transcript of dictation `entry_id` and
    /// keeps the transcript it replaces in the version's place, so nothing is
    /// lost and the count does not change.
    pub(super) fn restore_version_with_conn(
        conn: &mut Connection,
        entry_id: i64,
        version_id: i64,
    ) -> Result<()> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version = tx
            .query_row(
                "SELECT transcription_text, raw_text, cleanup, engine, fallback_reason,
                        produced_at
                   FROM transcription_versions WHERE id = ?1 AND entry_id = ?2",
                params![version_id, entry_id],
                |row| {
                    Ok(Transcript {
                        text: row.get(0)?,
                        raw_text: row.get(1)?,
                        cleanup: row.get(2)?,
                        engine: row.get(3)?,
                        fallback_reason: row.get(4)?,
                        produced_at: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("Version {} of entry {} not found", version_id, entry_id))?;
        let previous = current_transcript(&tx, entry_id)?;
        write_current(&tx, entry_id, &version)?;
        if previous.text.trim().is_empty() {
            tx.execute(
                "DELETE FROM transcription_versions WHERE id = ?1",
                params![version_id],
            )?;
        } else {
            tx.execute(
                "UPDATE transcription_versions
                    SET produced_at = ?1, transcription_text = ?2, raw_text = ?3,
                        cleanup = ?4, engine = ?5, fallback_reason = ?6
                  WHERE id = ?7",
                params![
                    previous.produced_at,
                    previous.text,
                    previous.raw_text,
                    previous.cleanup,
                    previous.engine,
                    previous.fallback_reason,
                    version_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Swaps an earlier transcript back in (see `restore_version_with_conn`)
    /// and tells the window.
    pub fn restore_version(&self, entry_id: i64, version_id: i64) -> Result<HistoryEntry> {
        let mut conn = self.get_connection()?;
        Self::restore_version_with_conn(&mut conn, entry_id, version_id)?;
        let entry = Self::query_entry(&conn, "WHERE id = ?1", params![entry_id])?
            .ok_or_else(|| anyhow!("History entry {} not found", entry_id))?;
        debug!(
            "Restored version {} of history entry {}",
            version_id, entry_id
        );
        self.emit_updated(&entry);
        Ok(entry)
    }
}
