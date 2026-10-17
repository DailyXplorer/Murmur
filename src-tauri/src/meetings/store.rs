//! `meetings.db`: one row per meeting, plus the chunk, speaker and turn tables
//! that processing fills in later. Migrations are append-only, as in
//! `history.rs`.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use log::warn;
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};

use super::paths;
use super::wav_recovery::repair_wav;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::managers::history::{open_or_reset_database, DatabaseStartup};

static MIGRATIONS: &[M] = &[M::up(
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
)];

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
    Migrations::new(MIGRATIONS.to_vec()).to_latest(&mut conn)?;
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
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO meetings (started_at_ms, capture_mode, status)
             VALUES (?1, ?2, 'recording')",
            params![started_at_ms, tracks.as_db()],
        )?;
        Ok(conn.last_insert_rowid())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

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
        Migrations::new(MIGRATIONS.to_vec()).validate().unwrap();
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
    fn a_meeting_cut_off_before_any_audio_is_recovered_with_no_duration() {
        let (dir, store) = store();
        let id = store.insert_recording(0, MeetingTracks::System).unwrap();
        assert_eq!(store.recover_interrupted(dir.path()).unwrap(), 1);
        let page = store.list(None, 1).unwrap();
        assert_eq!(page.meetings[0].id, id);
        assert_eq!(page.meetings[0].status, MeetingStatus::Interrupted);
        assert_eq!(page.meetings[0].duration_ms, 0);
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
}
