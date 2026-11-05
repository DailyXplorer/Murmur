use super::*;

#[cfg(unix)]
#[test]
fn retention_with_symlinked_recordings_root_keeps_external_sentinel_and_removes_database_row() {
    let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

    let deleted = HistoryManager::delete_entries_and_files_with_conn(
        &conn,
        &recordings_dir,
        &[(id, "murmur-100.wav".to_string())],
    )
    .expect("retain history entry with symlinked recordings root");

    assert_eq!(deleted, vec![id]);
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("read external sentinel"),
        "sentinel"
    );
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("count history entries");
    assert_eq!(remaining, 0);
}

#[test]
fn retention_of_invalid_recording_keeps_external_sentinel_and_removes_database_row() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let sentinel = temp.path().join("sentinel.wav");
    std::fs::write(&sentinel, "sentinel").expect("write sentinel");
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "../sentinel.wav");

    let deleted = HistoryManager::delete_entries_and_files_with_conn(
        &conn,
        &recordings_dir,
        &[(id, "../sentinel.wav".to_string())],
    )
    .expect("retain invalid history entry");

    assert_eq!(deleted, vec![id]);
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("read sentinel"),
        "sentinel"
    );
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("count history entries");
    assert_eq!(remaining, 0);
}

#[test]
fn retention_deletes_valid_recording_and_database_row() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let recording = recordings_dir.join("murmur-100.wav");
    std::fs::write(&recording, "wav").expect("write recording");
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

    let deleted = HistoryManager::delete_entries_and_files_with_conn(
        &conn,
        &recordings_dir,
        &[(id, "murmur-100.wav".to_string())],
    )
    .expect("retain valid history entry");

    assert_eq!(deleted, vec![id]);
    assert!(!recording.exists());
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("count history entries");
    assert_eq!(remaining, 0);
}

#[test]
fn retention_keeps_an_entry_starred_after_it_was_selected() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let recording = recordings_dir.join("murmur-100.wav");
    std::fs::write(&recording, "wav").expect("write recording");
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "murmur-100.wav");
    // The user stars the entry between the cleanup's SELECT and its DELETE.
    conn.execute(
        "UPDATE transcription_history SET saved = 1 WHERE id = ?1",
        params![id],
    )
    .expect("star history entry");

    let deleted = HistoryManager::delete_entries_and_files_with_conn(
        &conn,
        &recordings_dir,
        &[(id, "murmur-100.wav".to_string())],
    )
    .expect("retain history entries");

    assert!(deleted.is_empty());
    assert!(recording.exists());
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("count history entries");
    assert_eq!(remaining, 1);
}

#[test]
fn retention_delete_removes_dictations_from_search() {
    let (directory, conn) = history_db();
    let old = dictate(&conn, 5, "Old budget notes.");
    let kept = dictate(&conn, 9, "New budget notes.");

    let deleted = HistoryManager::delete_entries_and_files_with_conn(
        &conn,
        directory.path(),
        &[(old, "a.wav".to_string())],
    )
    .unwrap();

    assert_eq!(deleted, [old]);
    assert_eq!(found(&conn, "budget"), [kept]);
    let indexed: i64 = conn
        .query_row("SELECT count(*) FROM history_fts_docsize", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(indexed, 1);
}
