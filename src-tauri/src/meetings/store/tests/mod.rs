mod audio;
mod library;
mod migrations;
mod processing;
mod search;
mod summary;
mod transcript;

use super::*;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

use super::types::MeetingListItem;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::managers::history::DatabaseStartup;
use crate::meetings::capture::route::OutputRoute;
use crate::meetings::chunking::ChunkPlan;
use crate::meetings::merge::Track;
use crate::meetings::paths;
use crate::meetings::summary::SummaryError;
use crate::search::Snippet;

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

fn ready(store: &MeetingsStore) -> i64 {
    let id = recorded(store);
    store.begin_processing(id).unwrap();
    store.finish_processing(id, None).unwrap();
    id
}

fn me() -> MeetingSpeaker {
    MeetingSpeaker {
        key: "me".into(),
        name: "Me".into(),
        default_name: "Me".into(),
        color_index: 0,
    }
}

fn said(texts: &[&str]) -> Vec<MeetingTurn> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| MeetingTurn {
            speaker_key: "me".into(),
            start_ms: index as i64 * 1_000,
            end_ms: index as i64 * 1_000 + 900,
            text: text.to_string(),
            low_confidence: false,
        })
        .collect()
}

/// A processed meeting, with a Codex title and summary when given.
fn meeting(
    store: &MeetingsStore,
    started_at_ms: i64,
    turns: &[&str],
    summary: Option<(&str, &str)>,
) -> i64 {
    let id = store
        .insert_recording(started_at_ms, MeetingTracks::Mic)
        .unwrap();
    store
        .finish_recording(id, started_at_ms + 60_000, 60_000)
        .unwrap();
    store
        .save_transcript(
            id,
            &[me()],
            &said(turns),
            SpeakerAnalysis::NoModels,
            None,
            None,
        )
        .unwrap();
    store.finish_processing(id, None).unwrap();
    if let Some((title, body)) = summary {
        assert!(store.save_summary(id, Some(title), body).unwrap());
    }
    id
}

fn found(store: &MeetingsStore, query: &str) -> Vec<i64> {
    store
        .search(query, 50)
        .unwrap()
        .iter()
        .map(|hit| hit.id)
        .collect()
}

fn indexed_rows(conn: &Connection) -> (i64, i64) {
    let count = |table: &str| {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    (
        count("meetings_fts_docsize"),
        count("meeting_turns_fts_docsize"),
    )
}

fn marked(snippet: &Snippet) -> Vec<String> {
    let units: Vec<u16> = snippet.text.encode_utf16().collect();
    snippet
        .ranges
        .iter()
        .map(|(start, end)| String::from_utf16(&units[*start as usize..*end as usize]).unwrap())
        .collect()
}
