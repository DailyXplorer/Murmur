use anyhow::Result;
use log::{debug, info};
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use std::path::Path;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
pub(super) static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up(
        "ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;",
    ),
    M::up(
        "CREATE TABLE transcription_history_cloud_only (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );
        INSERT INTO transcription_history_cloud_only (
            id, file_name, timestamp, saved, title, transcription_text
        ) SELECT id, file_name, timestamp, saved, title, transcription_text
          FROM transcription_history;
        DROP TABLE transcription_history;
        ALTER TABLE transcription_history_cloud_only RENAME TO transcription_history;",
    ),
    M::up(
        "ALTER TABLE transcription_history ADD COLUMN raw_text TEXT;
        ALTER TABLE transcription_history ADD COLUMN cleanup TEXT;",
    ),
    M::up(
        "ALTER TABLE transcription_history ADD COLUMN engine TEXT NOT NULL DEFAULT 'cloud';
        ALTER TABLE transcription_history ADD COLUMN fallback_reason TEXT;",
    ),
    // Search, kept by these triggers only.
    M::up(
        "CREATE VIRTUAL TABLE history_fts USING fts5(
            transcription_text, content='transcription_history', content_rowid='id',
            tokenize='unicode61 remove_diacritics 2', prefix='2 3'
        );
        CREATE TRIGGER history_fts_insert AFTER INSERT ON transcription_history BEGIN
            INSERT INTO history_fts(rowid, transcription_text)
            VALUES (new.id, new.transcription_text);
        END;
        CREATE TRIGGER history_fts_delete AFTER DELETE ON transcription_history BEGIN
            INSERT INTO history_fts(history_fts, rowid, transcription_text)
            VALUES ('delete', old.id, old.transcription_text);
        END;
        CREATE TRIGGER history_fts_update AFTER UPDATE OF transcription_text
        ON transcription_history BEGIN
            INSERT INTO history_fts(history_fts, rowid, transcription_text)
            VALUES ('delete', old.id, old.transcription_text);
            INSERT INTO history_fts(rowid, transcription_text)
            VALUES (new.id, new.transcription_text);
        END;
        INSERT INTO history_fts(history_fts) VALUES ('rebuild');",
    ),
    // `ORDER BY timestamp DESC LIMIT 1` (last entry, paste last, search ties).
    M::up("CREATE INDEX IF NOT EXISTS idx_transcription_history_timestamp ON transcription_history(timestamp);"),
    // Transcripts a re-transcription replaced, at most five per dictation (the
    // cap is enforced in `versions.rs`). They are not in the search index,
    // which follows `transcription_history` only. The trigger removes them with
    // their dictation, whether or not foreign keys are enforced on the
    // connection. `transcribed_at` is when the current text was produced; rows
    // from before it read as the dictation's own time.
    M::up(
        "ALTER TABLE transcription_history ADD COLUMN transcribed_at INTEGER;
        CREATE TABLE transcription_versions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            entry_id INTEGER NOT NULL,
            produced_at INTEGER NOT NULL,
            transcription_text TEXT NOT NULL,
            raw_text TEXT,
            cleanup TEXT,
            engine TEXT NOT NULL,
            fallback_reason TEXT
        );
        CREATE INDEX transcription_versions_entry
            ON transcription_versions (entry_id, produced_at);
        CREATE TRIGGER transcription_versions_delete
        AFTER DELETE ON transcription_history BEGIN
            DELETE FROM transcription_versions WHERE entry_id = old.id;
        END;",
    ),
];

/// Converts the migration counter used by the former SQL plugin into SQLite's
/// `user_version` so already-applied schema changes are not replayed.
pub(super) fn migrate_from_tauri_plugin_sql(conn: &Connection) -> Result<()> {
    let has_legacy_tracking: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_legacy_tracking {
        return Ok(());
    }

    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current_version > 0 {
        return Ok(());
    }

    let legacy_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if legacy_version > 0 {
        info!(
            "Converting legacy history migration tracking at version {}",
            legacy_version
        );
        conn.pragma_update(None, "user_version", legacy_version)?;
    }

    Ok(())
}

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.to_vec())
}

pub(crate) fn init_database(db_path: &Path) -> Result<()> {
    info!("Initializing database at {:?}", db_path);

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

    migrate_from_tauri_plugin_sql(&conn)?;

    let migrations = migrations();

    #[cfg(debug_assertions)]
    migrations.validate().expect("Invalid migrations");

    let version_before: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    debug!("Database version before migration: {}", version_before);

    migrations.to_latest(&mut conn)?;

    let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if version_after > version_before {
        info!(
            "Database migrated from version {} to {}",
            version_before, version_after
        );
    } else {
        debug!("Database already at latest version {}", version_after);
    }

    Ok(())
}
