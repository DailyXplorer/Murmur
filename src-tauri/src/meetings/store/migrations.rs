use rusqlite::{params, Transaction};
use rusqlite_migration::{HookResult, Migrations, M};

use crate::meetings::summary::named_title;

pub(crate) fn migrations() -> Migrations<'static> {
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
        // Audio the track writers could not keep up with, recorded as silence.
        M::up("ALTER TABLE meetings ADD COLUMN dropped_ms INTEGER NOT NULL DEFAULT 0;"),
        // Failed attempts to compress the tracks; past a few, they stay.
        M::up("ALTER TABLE meetings ADD COLUMN compress_failures INTEGER NOT NULL DEFAULT 0;"),
        // Time the Mac slept while the meeting recorded, which holds no audio.
        M::up("ALTER TABLE meetings ADD COLUMN slept_ms INTEGER NOT NULL DEFAULT 0;"),
        // Unused: an unreleased build split a long meeting into linked rows.
        // Migrations only append, so the columns stay.
        M::up(
            "ALTER TABLE meetings ADD COLUMN previous_id INTEGER;
    ALTER TABLE meetings ADD COLUMN part INTEGER NOT NULL DEFAULT 1;",
        ),
        // Where the meeting played sound (`OutputRoute`); NULL for meetings
        // recorded before it was read.
        M::up("ALTER TABLE meetings ADD COLUMN output_route TEXT;"),
        // Search. The indexes are kept by these triggers only; the turns'
        // index points at their implicit rowid, which a VACUUM would
        // renumber, so one must be followed by a 'rebuild'.
        M::up(
            "CREATE VIRTUAL TABLE meeting_turns_fts USING fts5(
        text, content='meeting_turns', content_rowid='rowid',
        tokenize='unicode61 remove_diacritics 2', prefix='2 3'
    );
    CREATE TRIGGER meeting_turns_fts_insert AFTER INSERT ON meeting_turns BEGIN
        INSERT INTO meeting_turns_fts(rowid, text) VALUES (new.rowid, new.text);
    END;
    CREATE TRIGGER meeting_turns_fts_delete AFTER DELETE ON meeting_turns BEGIN
        INSERT INTO meeting_turns_fts(meeting_turns_fts, rowid, text)
        VALUES ('delete', old.rowid, old.text);
    END;
    CREATE TRIGGER meeting_turns_fts_update AFTER UPDATE OF text ON meeting_turns BEGIN
        INSERT INTO meeting_turns_fts(meeting_turns_fts, rowid, text)
        VALUES ('delete', old.rowid, old.text);
        INSERT INTO meeting_turns_fts(rowid, text) VALUES (new.rowid, new.text);
    END;
    CREATE VIRTUAL TABLE meetings_fts USING fts5(
        title, summary_md, content='meetings', content_rowid='id',
        tokenize='unicode61 remove_diacritics 2', prefix='2 3'
    );
    CREATE TRIGGER meetings_fts_insert AFTER INSERT ON meetings BEGIN
        INSERT INTO meetings_fts(rowid, title, summary_md)
        VALUES (new.id, new.title, new.summary_md);
    END;
    CREATE TRIGGER meetings_fts_delete AFTER DELETE ON meetings BEGIN
        INSERT INTO meetings_fts(meetings_fts, rowid, title, summary_md)
        VALUES ('delete', old.id, old.title, old.summary_md);
    END;
    CREATE TRIGGER meetings_fts_update AFTER UPDATE OF title, summary_md ON meetings BEGIN
        INSERT INTO meetings_fts(meetings_fts, rowid, title, summary_md)
        VALUES ('delete', old.id, old.title, old.summary_md);
        INSERT INTO meetings_fts(rowid, title, summary_md)
        VALUES (new.id, new.title, new.summary_md);
    END;
    INSERT INTO meeting_turns_fts(meeting_turns_fts) VALUES ('rebuild');
    INSERT INTO meetings_fts(meetings_fts) VALUES ('rebuild');",
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
