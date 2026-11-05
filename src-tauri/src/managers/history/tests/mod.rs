mod entries;
mod recordings;
mod retention;
mod startup;

use super::*;
use rusqlite::{params, Connection};

use chrono::{DateTime, Utc};
use rusqlite_migration::Migrations;
use std::path::PathBuf;

use super::entries::{cleanup_columns, insert_row};
use super::schema::{migrate_from_tauri_plugin_sql, MIGRATIONS};
use super::startup::set_aside_recordings;
use crate::cleanup::{CleanupOutcome, ProcessedText};
use crate::engine::{Engine, EngineUsed, FallbackReason};

fn startup_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL,
            raw_text TEXT,
            cleanup TEXT,
            engine TEXT NOT NULL DEFAULT 'cloud',
            fallback_reason TEXT
        );",
    )
    .expect("create transcription_history table");
    conn
}

fn insert_entry(conn: &Connection, timestamp: i64, text: &str) {
    conn.execute(
        "INSERT INTO transcription_history (
            file_name,
            timestamp,
            saved,
            title,
            transcription_text
        ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("murmur-{}.wav", timestamp),
            timestamp,
            false,
            format!("Recording {}", timestamp),
            text,
        ],
    )
    .expect("insert history entry");
}

fn insert_entry_with_file_name(conn: &Connection, file_name: &str) -> i64 {
    conn.execute(
        "INSERT INTO transcription_history (
            file_name,
            timestamp,
            saved,
            title,
            transcription_text
        ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![file_name, 100, false, "Recording 100", "transcription"],
    )
    .expect("insert history entry");
    conn.last_insert_rowid()
}

#[cfg(unix)]
fn symlinked_recordings_root_with_sentinel() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let external_recordings_dir = temp.path().join("external-recordings");
    std::fs::create_dir(&external_recordings_dir).expect("create external recordings directory");
    let sentinel = external_recordings_dir.join("murmur-100.wav");
    std::fs::write(&sentinel, "sentinel").expect("write external sentinel");
    let recordings_dir = temp.path().join("recordings");
    std::os::unix::fs::symlink(&external_recordings_dir, &recordings_dir)
        .expect("symlink recordings directory");

    (temp, recordings_dir, sentinel)
}

fn history_db() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    init_database(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    (directory, conn)
}

fn dictate(conn: &Connection, timestamp: i64, text: &str) -> i64 {
    let source = EngineUsed {
        engine: Engine::Cloud,
        fallback: None,
    };
    let text = ProcessedText::unprocessed(text.to_string());
    insert_row(conn, "a.wav", timestamp, "t", &text, source).unwrap()
}

fn found(conn: &Connection, query: &str) -> Vec<i64> {
    HistoryManager::search_with_conn(conn, query, 50)
        .unwrap()
        .iter()
        .map(|hit| hit.entry.id)
        .collect()
}
