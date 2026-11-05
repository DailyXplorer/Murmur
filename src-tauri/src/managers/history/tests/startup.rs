use super::*;

#[test]
fn corrupt_database_is_moved_aside_and_recreated() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let db_path = temp.path().join("history.db");
    std::fs::write(
        &db_path,
        b"this is not a sqlite database at all, just bytes",
    )
    .expect("write corrupt database");

    let startup = open_or_reset_database(&db_path, startup_time(), init_database);

    let moved_to = temp.path().join("history.db.corrupt-20261015-120000");
    assert_eq!(
        startup,
        DatabaseStartup::Reset {
            moved_to: moved_to.clone()
        }
    );
    assert_eq!(
        std::fs::read(&moved_to).expect("read moved database"),
        b"this is not a sqlite database at all, just bytes"
    );
    let conn = Connection::open(&db_path).expect("open recreated database");
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
            row.get(0)
        })
        .expect("query recreated history table");
    assert_eq!(rows, 0);
}

#[test]
fn the_recordings_of_a_reset_history_are_moved_next_to_it() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings = temp.path().join("recordings");
    std::fs::create_dir(&recordings).unwrap();
    std::fs::write(recordings.join("murmur-1.wav"), b"audio").unwrap();
    let moved_to = temp.path().join("history.db.corrupt-20261015-120000");

    set_aside_recordings(&recordings, &moved_to).unwrap();

    let aside = temp.path().join("recordings.corrupt-20261015-120000");
    assert_eq!(std::fs::read(aside.join("murmur-1.wav")).unwrap(), b"audio");
    assert_eq!(std::fs::read_dir(&recordings).unwrap().count(), 0);
}

#[test]
fn healthy_database_is_opened_in_place() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let db_path = temp.path().join("history.db");
    assert_eq!(
        open_or_reset_database(&db_path, startup_time(), init_database),
        DatabaseStartup::Ready
    );
    assert_eq!(
        open_or_reset_database(&db_path, startup_time(), init_database),
        DatabaseStartup::Ready
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn migration_adds_cleanup_columns_to_an_existing_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    init_database(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO transcription_history
            (file_name, timestamp, saved, title, transcription_text)
         VALUES ('a.wav', 5, 0, 't', 'old row')",
        [],
    )
    .unwrap();
    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .unwrap()
        .unwrap();
    assert_eq!(entry.raw_text, None);
    assert_eq!(entry.cleanup, None);
}

#[test]
fn migration_from_version_four_preserves_history_and_removes_retired_columns() {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL,
            post_processed_text TEXT,
            post_process_prompt TEXT,
            post_process_requested BOOLEAN NOT NULL DEFAULT 0
        );
        INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text,
            post_processed_text, post_process_prompt, post_process_requested
        ) VALUES (
            'murmur-100.wav', 100, 1, 'Recording 100', 'preserved',
            'retired output', 'retired prompt', 1
        );
        PRAGMA user_version = 4;",
    )
    .expect("create version-four history database");

    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .expect("upgrade version-four database");

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read migrated version");
    assert_eq!(version, 8);

    let columns = conn
        .prepare("PRAGMA table_info(transcription_history)")
        .expect("prepare table info")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table info")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect column names");
    assert_eq!(
        columns,
        [
            "id",
            "file_name",
            "timestamp",
            "saved",
            "title",
            "transcription_text",
            "raw_text",
            "cleanup",
            "engine",
            "fallback_reason"
        ]
    );

    let preserved: (String, bool) = conn
        .query_row(
            "SELECT transcription_text, saved FROM transcription_history WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read preserved history entry");
    assert_eq!(preserved, ("preserved".to_string(), true));
}

#[test]
fn fresh_migrations_create_only_the_cloud_transcription_schema() {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .expect("migrate fresh database");

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read migrated version");
    assert_eq!(version, 8);

    let columns = conn
        .prepare("PRAGMA table_info(transcription_history)")
        .expect("prepare table info")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query table info")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect column names");
    assert_eq!(
        columns,
        [
            "id",
            "file_name",
            "timestamp",
            "saved",
            "title",
            "transcription_text",
            "raw_text",
            "cleanup",
            "engine",
            "fallback_reason"
        ]
    );
}

#[test]
fn legacy_sql_plugin_tracking_is_converted_before_cloud_only_migration() {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE _sqlx_migrations (
            version INTEGER PRIMARY KEY,
            success BOOLEAN NOT NULL
        );
        INSERT INTO _sqlx_migrations (version, success)
            VALUES (1, 1), (2, 1), (3, 1), (4, 1);
        CREATE TABLE transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL,
            post_processed_text TEXT,
            post_process_prompt TEXT,
            post_process_requested BOOLEAN NOT NULL DEFAULT 0
        );",
    )
    .expect("create legacy SQL-plugin database");

    migrate_from_tauri_plugin_sql(&conn).expect("convert legacy tracking");
    let converted_version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read converted version");
    assert_eq!(converted_version, 4);

    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .expect("apply cloud-only migration");
    let final_version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read final version");
    assert_eq!(final_version, 8);
}
