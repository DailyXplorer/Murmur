use super::*;

#[test]
fn cleanup_columns_keep_the_raw_transcript_only_when_cleanup_changed_it() {
    let cleaned = ProcessedText {
        text: "So, 3 pm.".to_string(),
        raw: "so um 2 pm actually 3 pm".to_string(),
        cleanup: Some(CleanupOutcome::Cleaned),
    };
    assert_eq!(
        cleanup_columns(&cleaned),
        (
            Some("so um 2 pm actually 3 pm"),
            Some(CleanupOutcome::Cleaned)
        )
    );
    let failed = ProcessedText {
        text: "raw".to_string(),
        raw: "raw".to_string(),
        cleanup: Some(CleanupOutcome::Failed),
    };
    assert_eq!(
        cleanup_columns(&failed),
        (None, Some(CleanupOutcome::Failed))
    );
    assert_eq!(
        cleanup_columns(&ProcessedText::unprocessed("plain".to_string())),
        (None, None)
    );
}

#[test]
fn raw_text_and_cleanup_outcome_are_read_back() {
    let conn = setup_conn();
    conn.execute(
        "INSERT INTO transcription_history
            (file_name, timestamp, saved, title, transcription_text, raw_text, cleanup)
         VALUES ('a.wav', 5, 0, 't', 'Clean.', 'um clean', 'cleaned')",
        [],
    )
    .unwrap();
    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .unwrap()
        .unwrap();
    assert_eq!(entry.transcription_text, "Clean.");
    assert_eq!(entry.raw_text.as_deref(), Some("um clean"));
    assert_eq!(entry.cleanup, Some(CleanupOutcome::Cleaned));
}

#[test]
fn a_saved_row_keeps_its_engine_and_fallback_reason() {
    let conn = setup_conn();
    let text = ProcessedText {
        text: "Clean.".to_string(),
        raw: "um clean".to_string(),
        cleanup: Some(CleanupOutcome::Cleaned),
    };
    let source = EngineUsed {
        engine: Engine::Cloud,
        fallback: Some(FallbackReason::ModelNotReady),
    };
    let id = insert_row(&conn, "a.wav", 5, "t", &text, source).unwrap();

    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .unwrap()
        .unwrap();
    assert_eq!(entry.id, id);
    assert_eq!(entry.transcription_text, "Clean.");
    assert_eq!(entry.raw_text.as_deref(), Some("um clean"));
    assert_eq!(entry.cleanup, Some(CleanupOutcome::Cleaned));
    assert_eq!(entry.engine, Engine::Cloud);
    assert_eq!(entry.fallback_reason, Some(FallbackReason::ModelNotReady));
}

/// Playback and Retry find their recording through this lookup.
#[test]
fn an_entry_read_by_id_keeps_its_engine_and_fallback_reason() {
    let conn = setup_conn();
    let source = EngineUsed {
        engine: Engine::Cloud,
        fallback: Some(FallbackReason::ModelNotReady),
    };
    let text = ProcessedText::unprocessed("text".to_string());
    let id = insert_row(&conn, "a.wav", 5, "t", &text, source).unwrap();

    let entry = HistoryManager::get_entry_by_id_with_conn(&conn, id)
        .unwrap()
        .unwrap();
    assert_eq!(entry.file_name, "a.wav");
    assert_eq!(entry.engine, Engine::Cloud);
    assert_eq!(entry.fallback_reason, Some(FallbackReason::ModelNotReady));
    assert!(HistoryManager::get_entry_by_id_with_conn(&conn, id + 1)
        .unwrap()
        .is_none());
}

#[test]
fn rows_from_before_the_engine_columns_read_as_cloud_without_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.db");
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(MIGRATIONS[..6].to_vec())
        .to_latest(&mut conn)
        .unwrap();
    conn.execute(
        "INSERT INTO transcription_history
            (file_name, timestamp, saved, title, transcription_text)
         VALUES ('a.wav', 5, 0, 't', 'old row')",
        [],
    )
    .unwrap();
    drop(conn);

    init_database(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .unwrap()
        .unwrap();
    assert_eq!(entry.transcription_text, "old row");
    assert_eq!(entry.engine, Engine::Cloud);
    assert_eq!(entry.fallback_reason, None);
}

#[test]
fn an_unknown_engine_or_reason_written_by_a_newer_version_reads_safely() {
    let conn = setup_conn();
    conn.execute(
        "INSERT INTO transcription_history
            (file_name, timestamp, saved, title, transcription_text, engine, fallback_reason)
         VALUES ('a.wav', 5, 0, 't', 'text', 'quantum', 'solar_flare')",
        [],
    )
    .unwrap();
    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .unwrap()
        .unwrap();
    assert_eq!(entry.engine, Engine::Cloud);
    assert_eq!(entry.fallback_reason, None);
}

#[test]
fn get_latest_entry_returns_none_when_empty() {
    let conn = setup_conn();
    let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
    assert!(entry.is_none());
}

#[test]
fn get_latest_entry_returns_newest_entry() {
    let conn = setup_conn();
    insert_entry(&conn, 100, "first");
    insert_entry(&conn, 200, "second");

    let entry = HistoryManager::get_latest_entry_with_conn(&conn)
        .expect("fetch latest entry")
        .expect("entry exists");

    assert_eq!(entry.timestamp, 200);
    assert_eq!(entry.transcription_text, "second");
}

#[test]
fn get_latest_completed_entry_skips_empty_entries() {
    let conn = setup_conn();
    insert_entry(&conn, 100, "completed");
    insert_entry(&conn, 200, "");

    let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
        .expect("fetch latest completed entry")
        .expect("completed entry exists");

    assert_eq!(entry.timestamp, 100);
    assert_eq!(entry.transcription_text, "completed");
}

#[test]
fn frontend_requests_audio_by_history_entry_id() {
    let frontend =
        include_str!("../../../../../src/components/settings/history/HistorySettings.tsx");

    assert!(frontend.contains("const getAudioUrl = useCallback(async (id: number)"));
    assert!(frontend.contains("commands.getAudioFilePath(id)"));
    assert!(!frontend.contains("getAudioUrl(entry.file_name)"));
}

#[test]
fn history_search_finds_accented_word_without_accent() {
    let (_directory, conn) = history_db();
    let id = dictate(&conn, 5, "Prépare la réunion de lundi.");
    dictate(&conn, 6, "Rien à voir.");

    let hits = HistoryManager::search_with_conn(&conn, "reunion", 50).unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.id, id);
    let text: Vec<u16> = hits[0].entry.transcription_text.encode_utf16().collect();
    let (start, end) = hits[0].ranges[0];
    assert_eq!(
        String::from_utf16(&text[start as usize..end as usize]).unwrap(),
        "réunion"
    );
}

#[test]
fn history_search_prefix_matches() {
    let (_directory, conn) = history_db();
    let older = dictate(&conn, 5, "Send the pricing page.");
    let newer = dictate(&conn, 9, "The price list is out.");
    dictate(&conn, 7, "Nothing about money.");

    let mut prefixed = found(&conn, "pric");
    prefixed.sort();
    assert_eq!(prefixed, [older, newer]);
    assert_eq!(found(&conn, "pricing"), [older]);
    assert!(found(&conn, "pricey").is_empty());
}

fn clear(conn: &mut Connection, recordings_dir: &std::path::Path) -> usize {
    HistoryManager::clear_with_conn(conn, recordings_dir).unwrap()
}

#[test]
fn clearing_deletes_every_row_saved_ones_included_and_their_recordings() {
    let temp = tempfile::tempdir().unwrap();
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).unwrap();
    let (_db, mut conn) = history_db();
    let mut files = Vec::new();
    for (n, text) in ["first note", "second note", "third note"]
        .into_iter()
        .enumerate()
    {
        let file_name = format!("murmur-{n}.wav");
        std::fs::write(recordings_dir.join(&file_name), "wav").unwrap();
        let source = EngineUsed {
            engine: Engine::Cloud,
            fallback: None,
        };
        insert_row(
            &conn,
            &file_name,
            n as i64,
            "t",
            &ProcessedText::unprocessed(text.to_string()),
            source,
        )
        .unwrap();
        files.push(file_name);
    }
    conn.execute(
        "UPDATE transcription_history SET saved = 1 WHERE timestamp = 0",
        [],
    )
    .unwrap();

    assert_eq!(clear(&mut conn, &recordings_dir), 3);

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
    for file_name in &files {
        assert!(!recordings_dir.join(file_name).exists());
    }
    assert_eq!(clear(&mut conn, &recordings_dir), 0);
}

#[test]
fn clearing_keeps_the_search_index_in_step_with_the_rows() {
    let temp = tempfile::tempdir().unwrap();
    let (_db, mut conn) = history_db();
    dictate(&conn, 1, "quarterly budget review");
    dictate(&conn, 2, "budget for the offsite");
    assert_eq!(found(&conn, "budget").len(), 2);

    clear(&mut conn, temp.path());

    assert!(found(&conn, "budget").is_empty());
    conn.execute(
        "INSERT INTO history_fts(history_fts) VALUES ('integrity-check')",
        [],
    )
    .expect("the index matches the (now empty) table");
    // A dictation saved after the clear is found again.
    let id = dictate(&conn, 3, "budget again");
    assert_eq!(found(&conn, "budget"), vec![id]);
    conn.execute(
        "INSERT INTO history_fts(history_fts) VALUES ('integrity-check')",
        [],
    )
    .unwrap();
}

#[test]
fn clearing_leaves_a_recording_that_has_no_row_yet() {
    let temp = tempfile::tempdir().unwrap();
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).unwrap();
    std::fs::write(recordings_dir.join("in-flight.wav"), "wav").unwrap();
    std::fs::write(recordings_dir.join("a.wav"), "wav").unwrap();
    let (_db, mut conn) = history_db();
    dictate(&conn, 1, "hello");

    assert_eq!(clear(&mut conn, &recordings_dir), 1);

    assert!(!recordings_dir.join("a.wav").exists());
    assert!(recordings_dir.join("in-flight.wav").exists());
}

#[cfg(unix)]
#[test]
fn clearing_never_follows_a_symlinked_recordings_root() {
    let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();
    let (_db, mut conn) = history_db();
    let source = EngineUsed {
        engine: Engine::Cloud,
        fallback: None,
    };
    insert_row(
        &conn,
        "murmur-100.wav",
        100,
        "t",
        &ProcessedText::unprocessed("hello".to_string()),
        source,
    )
    .unwrap();

    assert_eq!(clear(&mut conn, &recordings_dir), 1);

    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "sentinel");
}
