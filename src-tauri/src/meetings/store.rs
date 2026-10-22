//! `meetings.db`: one row per meeting, plus the chunk, speaker and turn tables
//! that processing fills in later. Migrations are append-only, as in
//! `history.rs`.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use log::warn;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use rusqlite_migration::{HookResult, Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::chunking::ChunkPlan;
use super::merge::Track;
use super::paths;
use super::summary::{named_title, SummaryError};
use super::wav_recovery::repair_wav;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::managers::history::{open_or_reset_database, DatabaseStartup};

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE meetings (
        id              INTEGER PRIMARY KEY,
        started_at_ms   INTEGER NOT NULL,
        ended_at_ms     INTEGER,
        duration_ms     INTEGER NOT NULL DEFAULT 0,
        capture_mode    TEXT NOT NULL CHECK (capture_mode IN ('both','system','mic')),
        language        TEXT,
        title           TEXT,
        title_edited    INTEGER NOT NULL DEFAULT 0,
        status          TEXT NOT NULL CHECK (status IN
                        ('recording','interrupted','processing','ready','failed')),
        stage           TEXT,
        error_kind      TEXT,
        error_detail    TEXT,
        summary_md      TEXT,
        summary_status  TEXT NOT NULL DEFAULT 'pending'
                        CHECK (summary_status IN ('pending','ready','failed')),
        summary_error   TEXT,
        alignment_score REAL,
        audio_file      TEXT,
        audio_bytes     INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE meeting_chunks (
        meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
        track      TEXT NOT NULL CHECK (track IN ('mic','system')),
        idx        INTEGER NOT NULL,
        start_ms   INTEGER NOT NULL,
        end_ms     INTEGER NOT NULL,
        status     TEXT NOT NULL CHECK (status IN ('pending','done','failed','silent')),
        text       TEXT,
        attempts   INTEGER NOT NULL DEFAULT 0,
        error_kind TEXT,
        PRIMARY KEY (meeting_id, track, idx)
    );
    CREATE TABLE meeting_speakers (
        meeting_id   INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
        key          TEXT NOT NULL,
        default_name TEXT NOT NULL,
        display_name TEXT NOT NULL,
        color_index  INTEGER NOT NULL,
        PRIMARY KEY (meeting_id, key)
    );
    CREATE TABLE meeting_turns (
        meeting_id     INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
        seq            INTEGER NOT NULL,
        speaker_key    TEXT NOT NULL,
        start_ms       INTEGER NOT NULL,
        end_ms         INTEGER NOT NULL,
        text           TEXT NOT NULL,
        low_confidence INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (meeting_id, seq)
    );",
        ),
        // How the speakers of the last processing run were told apart.
        M::up("ALTER TABLE meetings ADD COLUMN speaker_analysis TEXT;"),
        // Which diarization the speaker keys come from (NULL: one per track).
        M::up("ALTER TABLE meetings ADD COLUMN speaker_identity TEXT;"),
        // The user deleted the audio; the text stays.
        M::up("ALTER TABLE meetings ADD COLUMN audio_deleted INTEGER NOT NULL DEFAULT 0;"),
        // System audio held only zeros: the permission is likely off.
        M::up("ALTER TABLE meetings ADD COLUMN system_audio_silent INTEGER NOT NULL DEFAULT 0;"),
        // Another app played sound while system audio was recorded.
        M::up("ALTER TABLE meetings ADD COLUMN system_played INTEGER NOT NULL DEFAULT 0;"),
        // Codex titles from 1.7.0 to 1.7.6 could hold speaker markers.
        M::up_with_hook("", name_speakers_in_titles),
        // The highest id ever given, as AUTOINCREMENT keeps it: the run of a
        // deleted meeting may still write by id, so no meeting reuses one.
        M::up(
            "CREATE TABLE meeting_last_id (id INTEGER NOT NULL);
    INSERT INTO meeting_last_id SELECT COALESCE(MAX(id), 0) FROM meetings;",
        ),
    ])
}

/// Gives the speaker markers left in Codex titles the speakers' names, as
/// `summary::parse` does for a new title. A title the user typed is theirs.
fn name_speakers_in_titles(tx: &Transaction) -> HookResult {
    let titles: Vec<(i64, String)> = tx
        .prepare("SELECT id, title FROM meetings WHERE title_edited = 0 AND title LIKE '%[[%'")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, title) in titles {
        let speakers: Vec<(String, String)> = tx
            .prepare("SELECT key, display_name FROM meeting_speakers WHERE meeting_id = ?1")?
            .query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let names = speakers
            .iter()
            .map(|(key, name)| (key.as_str(), name.clone()))
            .collect();
        tx.execute(
            "UPDATE meetings SET title = ?2 WHERE id = ?1",
            params![id, named_title(&title, &names)],
        )?;
    }
    Ok(())
}

/// Which tracks a meeting recorded.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingTracks {
    Both,
    System,
    Mic,
}

impl MeetingTracks {
    pub fn from_parts(mic: bool, system: bool) -> Option<Self> {
        match (mic, system) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Mic),
            (false, true) => Some(Self::System),
            (false, false) => None,
        }
    }

    pub fn has_mic(self) -> bool {
        self != Self::System
    }

    pub fn has_system(self) -> bool {
        self != Self::Mic
    }

    fn as_db(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::System => "system",
            Self::Mic => "mic",
        }
    }

    fn from_db(value: &str) -> Option<Self> {
        match value {
            "both" => Some(Self::Both),
            "system" => Some(Self::System),
            "mic" => Some(Self::Mic),
            _ => None,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Recording,
    /// The app quit or crashed while recording; the tracks were repaired.
    Interrupted,
    /// Recorded and waiting for (or in) processing.
    Processing,
    Ready,
    Failed,
}

impl MeetingStatus {
    fn from_db(value: &str) -> Option<Self> {
        match value {
            "recording" => Some(Self::Recording),
            "interrupted" => Some(Self::Interrupted),
            "processing" => Some(Self::Processing),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl MeetingStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Interrupted => "interrupted",
            Self::Processing => "processing",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// What processing is doing now.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStage {
    Transcribing,
    /// Telling speakers apart on device, once Codex is done.
    Analyzing,
    /// Placing the text of both tracks in order.
    Aligning,
}

impl MeetingStage {
    fn as_db(self) -> &'static str {
        match self {
            Self::Transcribing => "transcribing",
            Self::Analyzing => "analyzing",
            Self::Aligning => "aligning",
        }
    }

    fn from_db(value: &str) -> Option<Self> {
        match value {
            "transcribing" => Some(Self::Transcribing),
            "analyzing" => Some(Self::Analyzing),
            "aligning" => Some(Self::Aligning),
            _ => None,
        }
    }
}

/// How the speakers of a meeting were told apart.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerAnalysis {
    /// On-device diarization and word times were used.
    Separated,
    /// The models were not downloaded: one speaker per track.
    NoModels,
    /// The analysis failed or crashed: one speaker per track.
    Failed,
    /// This Mac cannot run the analysis: one speaker per track.
    Unsupported,
}

impl SpeakerAnalysis {
    fn as_db(self) -> &'static str {
        match self {
            Self::Separated => "separated",
            Self::NoModels => "no_models",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    fn from_db(value: &str) -> Option<Self> {
        match value {
            "separated" => Some(Self::Separated),
            "no_models" => Some(Self::NoModels),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Why the last processing run did not finish everything.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingError {
    /// Codex needs a new sign-in.
    SignedOut,
    /// Codex kept refusing requests for too long.
    RateLimited,
    /// Some parts could not be transcribed; the rest is shown.
    PartsFailed,
    /// No track could be read.
    NoAudio,
    /// Processing stopped on an unexpected error.
    Other,
}

impl ProcessingError {
    fn as_db(self) -> &'static str {
        match self {
            Self::SignedOut => "signed_out",
            Self::RateLimited => "rate_limited",
            Self::PartsFailed => "parts_failed",
            Self::NoAudio => "no_audio",
            Self::Other => "other",
        }
    }

    fn from_db(value: &str) -> Option<Self> {
        match value {
            "signed_out" => Some(Self::SignedOut),
            "rate_limited" => Some(Self::RateLimited),
            "parts_failed" => Some(Self::PartsFailed),
            "no_audio" => Some(Self::NoAudio),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SummaryStatus {
    /// Not written yet: it is once the whole transcript is.
    Pending,
    Ready,
    Failed,
}

impl SummaryStatus {
    fn from_db(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// Where a meeting's audio is.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    /// The tracks as recorded, until processing compresses them.
    Tracks,
    /// `audio.m4a`.
    Compressed,
    /// The user deleted it.
    Deleted,
}

fn track_db(track: Track) -> &'static str {
    match track {
        Track::Mic => "mic",
        Track::System => "system",
    }
}

fn track_from_db(value: &str) -> Option<Track> {
    match value {
        "mic" => Some(Track::Mic),
        "system" => Some(Track::System),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStatus {
    Pending,
    Done,
    Failed,
    /// No speech: never uploaded.
    Silent,
}

impl ChunkStatus {
    fn from_db(value: &str) -> Self {
        match value {
            "done" => Self::Done,
            "failed" => Self::Failed,
            "silent" => Self::Silent,
            _ => Self::Pending,
        }
    }
}

/// One piece of a track and what Codex made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRow {
    pub track: Track,
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: ChunkStatus,
    pub text: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct MeetingSpeaker {
    /// `me`, `s1`, `s2`…: stable, so a rename touches no text.
    pub key: String,
    pub name: String,
    pub default_name: String,
    pub color_index: u32,
}

/// One bubble of the transcript. A `_system` speaker marks a part that
/// could not be transcribed.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct MeetingTurn {
    pub speaker_key: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub low_confidence: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingDetail {
    pub id: i64,
    pub title: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub status: MeetingStatus,
    pub tracks: MeetingTracks,
    pub stage: Option<MeetingStage>,
    pub error: Option<ProcessingError>,
    /// `None` until the transcript is first built.
    pub speaker_analysis: Option<SpeakerAnalysis>,
    pub speakers: Vec<MeetingSpeaker>,
    pub turns: Vec<MeetingTurn>,
    /// Markdown with `[[key]]` markers where people are named.
    pub summary: Option<String>,
    pub summary_status: SummaryStatus,
    pub summary_error: Option<SummaryError>,
    pub audio: AudioState,
    /// The size of `audio.m4a`, once compressed.
    pub audio_bytes: i64,
    /// System audio was recorded as pure silence while the user spoke, which
    /// is what macOS gives when Murmur may not record system audio.
    pub system_audio_silent: bool,
    /// When a Codex rate-limit hold stops keeping this queued meeting
    /// waiting. Only the processing queue knows; the store never sets it.
    pub held_until_ms: Option<i64>,
    /// How much of the recorded audio this Mac transcribed because Codex
    /// refused it. "Transcribe again" sends it all to Codex once more.
    pub local_fallback_ms: i64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingListItem {
    pub id: i64,
    /// `None` until a summary names the meeting.
    pub title: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub status: MeetingStatus,
    pub tracks: MeetingTracks,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingsPage {
    pub meetings: Vec<MeetingListItem>,
    pub has_more: bool,
}

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

    pub fn insert_recording(&self, started_at_ms: i64, tracks: MeetingTracks) -> Result<i64> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE meeting_last_id
             SET id = MAX(id, (SELECT COALESCE(MAX(id), 0) FROM meetings)) + 1",
            [],
        )?;
        tx.execute(
            "INSERT INTO meetings (id, started_at_ms, capture_mode, status)
             VALUES ((SELECT id FROM meeting_last_id), ?1, ?2, 'recording')",
            params![started_at_ms, tracks.as_db()],
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
        let conn = self.connection()?;
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

    /// At startup no meeting can still be recording: one that is was cut off
    /// by a crash or a forced quit. Its track headers are repaired so the
    /// audio is readable, and it is marked interrupted with the duration the
    /// tracks hold. Returns how many meetings were recovered.
    pub fn recover_interrupted(&self, root: &Path) -> Result<usize> {
        let conn = self.connection()?;
        let cut_off = conn
            .prepare("SELECT id, capture_mode FROM meetings WHERE status = 'recording'")?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
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
            conn.execute(
                "UPDATE meetings SET status = 'interrupted', duration_ms = ?2 WHERE id = ?1",
                params![id, duration_ms as i64],
            )?;
        }
        Ok(cut_off.len())
    }

    /// Meetings to process, oldest first: those a quit left mid-way, and
    /// transcribed ones still waiting for their summary or compressed audio.
    pub fn waiting_for_processing(&self) -> Result<Vec<i64>> {
        Ok(self
            .connection()?
            .prepare(
                "SELECT id FROM meetings
                 WHERE status = 'processing'
                    OR (status = 'ready' AND (summary_status = 'pending'
                        OR (audio_file IS NULL AND audio_deleted = 0)))
                 ORDER BY id",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
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

    /// Saves the summary, and the title it gives the meeting unless the
    /// user named it. Returns false when the meeting no longer waits for
    /// it: transcribed again or re-analysed meanwhile, its summary is of a
    /// transcript that is gone.
    pub fn save_summary(&self, id: i64, title: Option<&str>, body: &str) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_md = ?3, summary_status = 'ready', summary_error = NULL,
                 title = CASE WHEN title_edited = 0 AND ?2 IS NOT NULL THEN ?2 ELSE title END
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'pending'",
            params![id, title, body],
        )?;
        Ok(updated > 0)
    }

    /// Returns false, like `save_summary`, when the meeting no longer waits
    /// for this summary.
    pub fn summary_failed(&self, id: i64, error: SummaryError) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_status = 'failed', summary_error = ?2
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'pending'",
            params![id, error.as_db()],
        )?;
        Ok(updated > 0)
    }

    /// Makes a failed summary wait to be written again. Returns false when
    /// it did not fail, or when the meeting is not Ready: only a Ready
    /// meeting gets its summary written.
    pub fn retry_summary(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_status = 'pending', summary_error = NULL
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'failed'",
            params![id],
        )?;
        Ok(updated > 0)
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

    /// Marks a finished meeting's audio deleted; its text stays. Returns
    /// false while it records or processes.
    pub fn delete_audio(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET audio_deleted = 1, audio_file = NULL, audio_bytes = 0
             WHERE id = ?1 AND status IN ('ready', 'failed')",
            params![id],
        )?;
        Ok(updated > 0)
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
    /// speakers apart anew. Returns false when it is not finished.
    pub fn reopen(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET status = 'processing', stage = NULL
             WHERE id = ?1 AND status IN ('ready', 'failed')",
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
    pub fn finish_processing(&self, id: i64, error: Option<ProcessingError>) -> Result<()> {
        let status = if error.is_some() {
            MeetingStatus::Failed
        } else {
            MeetingStatus::Ready
        };
        self.connection()?.execute(
            "UPDATE meetings SET status = ?2, stage = NULL, error_kind = ?3
             WHERE id = ?1 AND status = 'processing'",
            params![id, status.as_db(), error.map(ProcessingError::as_db)],
        )?;
        Ok(())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-17T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn store() -> (tempfile::TempDir, MeetingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let (store, startup) = MeetingsStore::open(dir.path().join("meetings.db"), now());
        assert_eq!(startup, DatabaseStartup::Ready);
        (dir, store)
    }

    fn status_of(store: &MeetingsStore, id: i64) -> Option<MeetingStatus> {
        let status: Option<String> = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT status FROM meetings WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        status.as_deref().and_then(MeetingStatus::from_db)
    }

    /// A track as a crash leaves it: the header counts no samples, but
    /// `samples` reached the disk after it.
    fn write_track(path: &Path, samples: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::create(path, spec)
            .unwrap()
            .finalize()
            .unwrap();
        let mut bytes = std::fs::read(path).unwrap();
        bytes.extend(vec![0u8; samples * 2]);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn migrations_are_valid() {
        migrations().validate().unwrap();
    }

    #[test]
    fn a_recording_is_listed_newest_first_once_finished() {
        let (_dir, store) = store();
        let first = store.insert_recording(1_000, MeetingTracks::Both).unwrap();
        let second = store.insert_recording(5_000, MeetingTracks::Mic).unwrap();
        store.finish_recording(first, 4_000, 3_000).unwrap();

        let page = store.list(None, 10).unwrap();
        assert!(!page.has_more);
        assert_eq!(
            page.meetings,
            vec![
                MeetingListItem {
                    id: second,
                    title: None,
                    started_at_ms: 5_000,
                    duration_ms: 0,
                    status: MeetingStatus::Recording,
                    tracks: MeetingTracks::Mic,
                },
                MeetingListItem {
                    id: first,
                    title: None,
                    started_at_ms: 1_000,
                    duration_ms: 3_000,
                    status: MeetingStatus::Processing,
                    tracks: MeetingTracks::Both,
                },
            ]
        );
        assert!(store.finish_recording(first, 9_000, 8_000).is_err());
    }

    #[test]
    fn pages_continue_after_the_cursor() {
        let (_dir, store) = store();
        let ids: Vec<i64> = (0..5)
            .map(|index| store.insert_recording(index, MeetingTracks::Both).unwrap())
            .collect();

        let first = store.list(None, 2).unwrap();
        assert!(first.has_more);
        assert_eq!(
            first.meetings.iter().map(|m| m.id).collect::<Vec<_>>(),
            [ids[4], ids[3]]
        );
        let last = store.list(Some(ids[1]), 2).unwrap();
        assert!(!last.has_more);
        assert_eq!(
            last.meetings.iter().map(|m| m.id).collect::<Vec<_>>(),
            [ids[0]]
        );
    }

    #[test]
    fn tracks_can_be_narrowed_after_a_failed_source() {
        let (_dir, store) = store();
        let id = store.insert_recording(0, MeetingTracks::Both).unwrap();
        store.set_tracks(id, MeetingTracks::Mic).unwrap();
        assert_eq!(
            store.list(None, 1).unwrap().meetings[0].tracks,
            MeetingTracks::Mic
        );
    }

    #[test]
    fn deleting_a_meeting_cascades_to_its_rows() {
        let (_dir, store) = store();
        let id = store.insert_recording(0, MeetingTracks::Both).unwrap();
        let conn = store.connection().unwrap();
        conn.execute(
            "INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text)
             VALUES (?1, 0, 'me', 0, 10, 'hello')",
            params![id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meeting_speakers (meeting_id, key, default_name, display_name, color_index)
             VALUES (?1, 'me', 'Me', 'Me', 0)",
            params![id],
        )
        .unwrap();

        store.delete(id).unwrap();

        for table in ["meetings", "meeting_turns", "meeting_speakers"] {
            let rows: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
    }

    #[test]
    fn a_meeting_cut_off_by_a_crash_is_recovered_as_interrupted() {
        let (dir, store) = store();
        let root = dir.path().join("meetings");
        let crashed = store.insert_recording(0, MeetingTracks::Both).unwrap();
        let finished = store.insert_recording(1, MeetingTracks::Mic).unwrap();
        store.finish_recording(finished, 2, 1_000).unwrap();
        let track_dir = paths::meeting_dir(&root, crashed);
        write_track(&track_dir.join(paths::MIC_TRACK), 32_000);
        write_track(&track_dir.join(paths::SYSTEM_TRACK), 31_840);

        assert_eq!(store.recover_interrupted(&root).unwrap(), 1);

        assert_eq!(status_of(&store, crashed), Some(MeetingStatus::Interrupted));
        assert_eq!(status_of(&store, finished), Some(MeetingStatus::Processing));
        let page = store.list(None, 10).unwrap();
        let recovered = page.meetings.iter().find(|m| m.id == crashed).unwrap();
        assert_eq!(recovered.duration_ms, 2_000);
        for file in [paths::MIC_TRACK, paths::SYSTEM_TRACK] {
            let reader = hound::WavReader::open(track_dir.join(file)).unwrap();
            assert!(reader.duration() >= 31_840);
        }
        assert_eq!(store.recover_interrupted(&root).unwrap(), 0);
    }

    #[test]
    fn a_retried_meeting_waits_for_processing_at_once() {
        let (dir, store) = store();
        let failed = recorded(&store);
        store.begin_processing(failed).unwrap();
        store
            .finish_processing(failed, Some(ProcessingError::SignedOut))
            .unwrap();
        let interrupted = store.insert_recording(1, MeetingTracks::Mic).unwrap();
        store.recover_interrupted(dir.path()).unwrap();

        // Another meeting is processing: the worker takes these later.
        for id in [failed, interrupted] {
            assert!(store.wait_for_retry(id).unwrap());
            let meeting = store.get(id).unwrap().unwrap();
            assert_eq!(meeting.status, MeetingStatus::Processing);
            assert_eq!(meeting.stage, None);
            assert!(!store.wait_for_retry(id).unwrap());
        }
        // A quit before their turn processes them at the next launch.
        assert_eq!(
            store.waiting_for_processing().unwrap(),
            [failed, interrupted]
        );
        assert!(store.begin_processing(failed).unwrap());
        assert_eq!(store.get(failed).unwrap().unwrap().error, None);
        store.finish_processing(failed, None).unwrap();
        assert!(!store.wait_for_retry(failed).unwrap(), "ready");
    }

    #[test]
    fn a_meeting_cut_off_before_any_audio_is_recovered_with_no_duration() {
        let (dir, store) = store();
        let id = store.insert_recording(0, MeetingTracks::System).unwrap();
        assert_eq!(store.recover_interrupted(dir.path()).unwrap(), 1);
        let page = store.list(None, 1).unwrap();
        assert_eq!(page.meetings[0].id, id);
        assert_eq!(page.meetings[0].status, MeetingStatus::Interrupted);
        assert_eq!(page.meetings[0].duration_ms, 0);
    }

    fn recorded(store: &MeetingsStore) -> i64 {
        let id = store.insert_recording(1_000, MeetingTracks::Both).unwrap();
        store.finish_recording(id, 61_000, 60_000).unwrap();
        id
    }

    fn plan(spans: &[(u64, u64, u64)]) -> Vec<ChunkPlan> {
        spans
            .iter()
            .map(|(start_ms, end_ms, voiced_ms)| ChunkPlan {
                start_ms: *start_ms,
                end_ms: *end_ms,
                voiced_ms: *voiced_ms,
            })
            .collect()
    }

    #[test]
    fn a_chunk_plan_is_stored_once_and_keeps_its_results() {
        let (_dir, store) = store();
        let id = recorded(&store);
        store
            .plan_chunks(
                id,
                Track::Mic,
                &plan(&[(0, 600_000, 90_000), (600_000, 700_000, 0)]),
            )
            .unwrap();
        store
            .plan_chunks(id, Track::System, &plan(&[(0, 700_000, 5_000)]))
            .unwrap();
        store.chunk_done(id, Track::Mic, 0, "Bonjour.").unwrap();
        store.chunk_failed(id, Track::System, 0, "network").unwrap();
        // Planning again after a restart changes nothing.
        store
            .plan_chunks(id, Track::Mic, &plan(&[(0, 700_000, 90_000)]))
            .unwrap();

        let chunks = store.chunks(id).unwrap();
        let summary: Vec<_> = chunks
            .iter()
            .map(|c| (c.track, c.idx, c.start_ms, c.status, c.text.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                (Track::System, 0, 0, ChunkStatus::Failed, None),
                (Track::Mic, 0, 0, ChunkStatus::Done, Some("Bonjour.")),
                (Track::Mic, 1, 600_000, ChunkStatus::Silent, None),
            ]
        );
    }

    /// A run of a deleted meeting must not write into a meeting that is
    /// still recording.
    #[test]
    fn only_a_meeting_being_processed_gets_a_plan_or_a_silence_flag() {
        let (_dir, store) = store();
        let live = store.insert_recording(0, MeetingTracks::Both).unwrap();
        store
            .plan_chunks(live, Track::Mic, &plan(&[(0, 60_000, 30_000)]))
            .unwrap();
        store.set_system_audio_silent(live, true).unwrap();
        assert!(store.chunks(live).unwrap().is_empty());
        assert!(!store.get(live).unwrap().unwrap().system_audio_silent);
    }

    /// The run of a deleted meeting may still be saving its transcript: the
    /// next meeting must not take its id, nor its folder.
    #[test]
    fn a_deleted_meeting_id_is_never_given_again() {
        let (_dir, store) = store();
        let kept = recorded(&store);
        let deleted = recorded(&store);
        store.delete(deleted).unwrap();
        let next = store.insert_recording(0, MeetingTracks::Both).unwrap();
        assert!(next > deleted && kept < deleted);
    }

    #[test]
    fn only_a_meeting_being_processed_gets_a_transcript_or_a_stage() {
        let (_dir, store) = store();
        let live = store.insert_recording(0, MeetingTracks::Mic).unwrap();
        let me = MeetingSpeaker {
            key: "me".into(),
            name: "Me".into(),
            default_name: "Me".into(),
            color_index: 0,
        };
        let turn = MeetingTurn {
            speaker_key: "me".into(),
            start_ms: 0,
            end_ms: 1_000,
            text: "Deleted.".into(),
            low_confidence: false,
        };
        store
            .save_transcript(live, &[me], &[turn], SpeakerAnalysis::Separated, None, None)
            .unwrap();
        store.set_stage(live, MeetingStage::Aligning).unwrap();
        let detail = store.get(live).unwrap().unwrap();
        assert!(detail.turns.is_empty() && detail.speakers.is_empty());
        assert_eq!((detail.stage, detail.speaker_analysis), (None, None));
    }

    #[test]
    fn processing_runs_only_on_recorded_meetings_and_records_its_outcome() {
        let (_dir, store) = store();
        let live = store.insert_recording(0, MeetingTracks::Mic).unwrap();
        assert!(!store.begin_processing(live).unwrap());
        assert!(!store.begin_processing(999).unwrap());

        let id = recorded(&store);
        assert_eq!(store.waiting_for_processing().unwrap(), [id]);
        assert!(store.begin_processing(id).unwrap());
        store.set_stage(id, MeetingStage::Aligning).unwrap();
        assert_eq!(
            store.get(id).unwrap().unwrap().stage,
            Some(MeetingStage::Aligning)
        );
        store
            .finish_processing(id, Some(ProcessingError::SignedOut))
            .unwrap();
        let failed = store.get(id).unwrap().unwrap();
        assert_eq!(failed.status, MeetingStatus::Failed);
        assert_eq!(failed.error, Some(ProcessingError::SignedOut));
        assert_eq!(failed.stage, None);
        assert!(store.waiting_for_processing().unwrap().is_empty());

        // Retrying clears the error; success makes it ready, for good.
        assert!(store.begin_processing(id).unwrap());
        assert_eq!(store.get(id).unwrap().unwrap().error, None);
        store.finish_processing(id, None).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().status, MeetingStatus::Ready);
        assert!(!store.begin_processing(id).unwrap());
    }

    #[test]
    fn a_saved_transcript_is_read_back_in_order_and_keeps_renamed_speakers() {
        let (_dir, store) = store();
        let id = recorded(&store);
        let speakers = vec![
            MeetingSpeaker {
                key: "me".into(),
                name: "Me".into(),
                default_name: "Me".into(),
                color_index: 0,
            },
            MeetingSpeaker {
                key: "s1".into(),
                name: "Speaker 2".into(),
                default_name: "Speaker 2".into(),
                color_index: 1,
            },
        ];
        let turn = |key: &str, start_ms: i64, text: &str| MeetingTurn {
            speaker_key: key.into(),
            start_ms,
            end_ms: start_ms + 1_000,
            text: text.into(),
            low_confidence: false,
        };
        store
            .save_transcript(
                id,
                &speakers,
                &[turn("s1", 0, "Salut."), turn("me", 900, "Hello.")],
                SpeakerAnalysis::NoModels,
                None,
                None,
            )
            .unwrap();
        store.rename_speaker(id, "s1", "Sam").unwrap();
        store.rename_speaker(id, "me", "Host").unwrap();
        assert!(store.rename_speaker(id, "s9", "Nobody").is_err());

        // Processing again from the same speakers replaces the turns but
        // keeps the renames.
        store
            .save_transcript(
                id,
                &speakers,
                &[turn("s1", 0, "Salut tout le monde.")],
                SpeakerAnalysis::Failed,
                None,
                Some(0.9),
            )
            .unwrap();
        let detail = store.get(id).unwrap().unwrap();
        assert_eq!(detail.speakers[1].name, "Sam");
        assert_eq!(detail.speakers[1].default_name, "Speaker 2");
        assert_eq!(detail.turns, [turn("s1", 0, "Salut tout le monde.")]);
        assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Failed));

        // A new diarization may give "s1" to someone else: only the user
        // keeps their name.
        store
            .save_transcript(
                id,
                &speakers,
                &[turn("s1", 0, "Salut.")],
                SpeakerAnalysis::Separated,
                Some("first"),
                None,
            )
            .unwrap();
        let detail = store.get(id).unwrap().unwrap();
        assert_eq!(detail.speakers[0].name, "Host");
        assert_eq!(detail.speakers[1].name, "Speaker 2");
        assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Separated));

        // A name the user never changed follows a new default.
        let renumbered = [
            speakers[0].clone(),
            MeetingSpeaker {
                name: "Speaker 3".into(),
                default_name: "Speaker 3".into(),
                ..speakers[1].clone()
            },
            MeetingSpeaker {
                key: "s2".into(),
                name: "Speaker 2".into(),
                default_name: "Speaker 2".into(),
                color_index: 2,
            },
        ];
        store
            .save_transcript(
                id,
                &renumbered,
                &[turn("s1", 0, "Salut.")],
                SpeakerAnalysis::Separated,
                Some("first"),
                None,
            )
            .unwrap();
        let detail = store.get(id).unwrap().unwrap();
        assert_eq!(detail.speakers[1].name, "Speaker 3");
        assert_eq!(detail.speakers[2].name, "Speaker 2");

        // A speaker the new analysis no longer finds is gone.
        store
            .save_transcript(
                id,
                &speakers[..1],
                &[turn("me", 0, "Seul.")],
                SpeakerAnalysis::Failed,
                None,
                None,
            )
            .unwrap();
        let detail = store.get(id).unwrap().unwrap();
        assert_eq!(detail.speakers.len(), 1);
        assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Failed));
    }

    #[test]
    fn only_a_finished_meeting_can_be_reopened() {
        let (_dir, store) = store();
        let id = recorded(&store);
        assert!(!store.reopen(id).unwrap());
        store.begin_processing(id).unwrap();
        store.finish_processing(id, None).unwrap();
        assert!(store.reopen(id).unwrap());
        assert_eq!(
            store.get(id).unwrap().unwrap().status,
            MeetingStatus::Processing
        );
        assert!(store.begin_processing(id).unwrap());
    }

    #[test]
    fn a_meeting_can_be_renamed_and_given_back_its_default_name() {
        let (_dir, store) = store();
        let id = recorded(&store);
        store.rename(id, Some("Point hebdo")).unwrap();
        assert_eq!(
            store.get(id).unwrap().unwrap().title.as_deref(),
            Some("Point hebdo")
        );
        assert_eq!(
            store.list(None, 1).unwrap().meetings[0].title.as_deref(),
            Some("Point hebdo")
        );
        store.rename(id, None).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().title, None);
        assert!(store.rename(999, Some("x")).is_err());
        assert_eq!(store.get(999).unwrap(), None);
    }

    fn ready(store: &MeetingsStore) -> i64 {
        let id = recorded(store);
        store.begin_processing(id).unwrap();
        store.finish_processing(id, None).unwrap();
        id
    }

    #[test]
    fn the_summary_names_the_meeting_unless_the_user_did() {
        let (_dir, store) = store();
        let id = ready(&store);
        let fresh = store.get(id).unwrap().unwrap();
        assert_eq!(fresh.summary_status, SummaryStatus::Pending);
        assert_eq!(fresh.summary, None);

        assert!(store
            .save_summary(id, Some("Budget review"), "Notes about [[me]].")
            .unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.title.as_deref(), Some("Budget review"));
        assert_eq!(meeting.summary.as_deref(), Some("Notes about [[me]]."));
        assert_eq!(meeting.summary_status, SummaryStatus::Ready);
        // Written once: a second summary is not waited for.
        assert!(!store.save_summary(id, Some("Other"), "Other.").unwrap());
        assert_eq!(
            store.get(id).unwrap().unwrap().summary.as_deref(),
            Some("Notes about [[me]].")
        );

        let named = ready(&store);
        store.rename(named, Some("My title")).unwrap();
        assert!(store
            .save_summary(named, Some("Other"), "New notes.")
            .unwrap());
        let meeting = store.get(named).unwrap().unwrap();
        assert_eq!(meeting.title.as_deref(), Some("My title"));
        assert_eq!(meeting.summary.as_deref(), Some("New notes."));

        let untitled = ready(&store);
        assert!(store.save_summary(untitled, None, "Notes.").unwrap());
        assert_eq!(store.get(untitled).unwrap().unwrap().title, None);
    }

    #[test]
    fn a_failed_summary_waits_for_retry_and_then_for_processing() {
        let (_dir, store) = store();
        let id = ready(&store);
        store.audio_compressed(id, 1_000).unwrap();
        // Pending: processing writes it, at the next launch too.
        assert_eq!(store.waiting_for_processing().unwrap(), [id]);
        assert!(!store.retry_summary(id).unwrap());

        store.summary_failed(id, SummaryError::RateLimited).unwrap();
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary_status, SummaryStatus::Failed);
        assert_eq!(meeting.summary_error, Some(SummaryError::RateLimited));
        assert!(store.waiting_for_processing().unwrap().is_empty());

        assert!(store.retry_summary(id).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary_status, SummaryStatus::Pending);
        assert_eq!(meeting.summary_error, None);
        assert_eq!(store.waiting_for_processing().unwrap(), [id]);

        // A run that ends Failed keeps the failed summary, which only the
        // meeting's own retry can bring back.
        store.summary_failed(id, SummaryError::RateLimited).unwrap();
        assert!(store.reopen(id).unwrap());
        store
            .finish_processing(id, Some(ProcessingError::NoAudio))
            .unwrap();
        assert!(!store.retry_summary(id).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary_status, SummaryStatus::Failed);
    }

    #[test]
    fn audio_is_compressed_then_deleted_and_the_text_stays() {
        let (_dir, store) = store();
        let recording = store.insert_recording(0, MeetingTracks::Mic).unwrap();
        assert!(!store.delete_audio(recording).unwrap());

        let id = ready(&store);
        store.save_summary(id, None, "Notes.").unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().audio, AudioState::Tracks);
        // Waiting for its audio to be compressed.
        assert_eq!(store.waiting_for_processing().unwrap(), [id]);

        assert!(store.audio_compressed(id, 4_200).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.audio, AudioState::Compressed);
        assert_eq!(meeting.audio_bytes, 4_200);
        assert!(store.waiting_for_processing().unwrap().is_empty());

        assert!(store.delete_audio(id).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.audio, AudioState::Deleted);
        assert_eq!(meeting.audio_bytes, 0);
        assert_eq!(meeting.summary.as_deref(), Some("Notes."));
        assert!(!store.audio_compressed(id, 4_200).unwrap());
        assert!(store.waiting_for_processing().unwrap().is_empty());
        assert!(!store.retranscribe(id).unwrap());
    }

    #[test]
    fn transcribing_again_forgets_the_pieces_and_the_summary_goes_with_the_words() {
        let (_dir, store) = store();
        let id = recorded(&store);
        store
            .plan_chunks(id, Track::Mic, &plan(&[(0, 60_000, 30_000)]))
            .unwrap();
        store.chunk_done(id, Track::Mic, 0, "Bonjour.").unwrap();
        let transcript = |text: &str| {
            store
                .save_transcript(
                    id,
                    &[],
                    &[MeetingTurn {
                        speaker_key: "me".into(),
                        start_ms: 0,
                        end_ms: 1_000,
                        text: text.into(),
                        low_confidence: false,
                    }],
                    SpeakerAnalysis::NoModels,
                    None,
                    None,
                )
                .unwrap()
        };
        transcript("Bonjour.");
        store.finish_processing(id, None).unwrap();
        store.rename(id, Some("Kept")).unwrap();
        store.save_summary(id, Some("Ignored"), "Notes.").unwrap();

        assert!(store.retranscribe(id).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Processing);
        // The audio may turn out unreadable: the summary waits for the
        // new transcript.
        assert_eq!(meeting.summary.as_deref(), Some("Notes."));
        assert!(store.chunks(id).unwrap().is_empty());
        // Already waiting: a second request changes nothing.
        assert!(!store.retranscribe(id).unwrap());

        // The same words keep their summary; other words get a new one.
        transcript("Bonjour.");
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary_status, SummaryStatus::Ready);
        transcript("Bonsoir.");
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary, None);
        assert_eq!(meeting.summary_status, SummaryStatus::Pending);
        assert_eq!(meeting.title.as_deref(), Some("Kept"));
    }

    #[test]
    fn a_summary_of_the_old_transcript_is_dropped_after_transcribing_again() {
        let (_dir, store) = store();
        let id = ready(&store);
        // Codex is still writing the summary when the user transcribes again.
        assert!(store.retranscribe(id).unwrap());
        assert!(!store
            .save_summary(id, Some("Old title"), "Old notes.")
            .unwrap());
        assert!(!store.summary_failed(id, SummaryError::Timeout).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary, None);
        assert_eq!(meeting.summary_status, SummaryStatus::Pending);
        assert_eq!(meeting.summary_error, None);
        assert_eq!(meeting.title, None);

        // The new transcript gets its own summary.
        store.finish_processing(id, None).unwrap();
        assert!(store
            .save_summary(id, Some("New title"), "New notes.")
            .unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary.as_deref(), Some("New notes."));
        assert_eq!(meeting.title.as_deref(), Some("New title"));
    }

    #[test]
    fn new_speaker_keys_make_the_summary_be_written_again() {
        let (_dir, store) = store();
        let id = recorded(&store);
        let speakers = [MeetingSpeaker {
            key: "s1".into(),
            name: "Speaker 1".into(),
            default_name: "Speaker 1".into(),
            color_index: 1,
        }];
        let save = |identity: Option<&str>| {
            store
                .save_transcript(
                    id,
                    &speakers,
                    &[],
                    SpeakerAnalysis::Separated,
                    identity,
                    None,
                )
                .unwrap()
        };
        save(None);
        store.finish_processing(id, None).unwrap();
        store.save_summary(id, None, "Notes by [[s1]].").unwrap();
        // The same keys keep the summary.
        assert!(store.reopen(id).unwrap());
        save(None);
        assert_eq!(
            store.get(id).unwrap().unwrap().summary_status,
            SummaryStatus::Ready
        );
        save(Some("diarized"));
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.summary_status, SummaryStatus::Pending);
        assert_eq!(meeting.summary, None);
    }

    #[test]
    fn a_corrupt_database_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        std::fs::write(&path, b"definitely not sqlite, only some bytes here").unwrap();
        let (store, startup) = MeetingsStore::open(path, now());
        assert_eq!(
            startup,
            DatabaseStartup::Reset {
                moved_to: dir.path().join("meetings.db.corrupt-20261017-090000")
            }
        );
        assert!(store.list(None, 1).unwrap().meetings.is_empty());
    }

    /// Codex titles from 1.7.0 to 1.7.6 could hold speaker markers; the
    /// first launch after them gives those titles the speakers' names.
    #[test]
    fn titles_saved_with_speaker_markers_get_the_speakers_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let (store, _) = MeetingsStore::open(path.clone(), now());
        let speaker = |key: &str, name: &str, color_index| MeetingSpeaker {
            key: key.into(),
            name: name.into(),
            default_name: name.into(),
            color_index,
        };
        let speakers = [speaker("me", "Me", 0), speaker("s1", "Speaker 1", 1)];
        let titled = |title: &str| {
            let id = recorded(&store);
            store
                .save_transcript(id, &speakers, &[], SpeakerAnalysis::Separated, None, None)
                .unwrap();
            store.finish_processing(id, None).unwrap();
            store.rename_speaker(id, "s1", "Camille").unwrap();
            store.save_summary(id, Some(title), "Notes.").unwrap();
            id
        };
        let written = titled("Point avec [[s1]] et [[me]], [[s9]]");
        let edited = titled("Brouillon");
        store.rename(edited, Some("Notes sur [[s1]]")).unwrap();
        // The library as 1.7.6 left it.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE meeting_last_id; PRAGMA user_version = 6;")
            .unwrap();
        drop(conn);

        let (store, startup) = MeetingsStore::open(path, now());
        assert_eq!(startup, DatabaseStartup::Ready);
        let title = |id| store.get(id).unwrap().unwrap().title;
        assert_eq!(
            title(written).as_deref(),
            Some("Point avec Camille et Me, s9")
        );
        // A title the user typed is theirs.
        assert_eq!(title(edited).as_deref(), Some("Notes sur [[s1]]"));
    }
}
