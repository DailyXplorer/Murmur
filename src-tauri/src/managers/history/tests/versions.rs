use super::*;
use crate::managers::history::versions::MAX_VERSIONS;

fn on_this_computer() -> EngineUsed {
    EngineUsed {
        engine: Engine::Local,
        fallback: None,
    }
}

fn cloud() -> EngineUsed {
    EngineUsed {
        engine: Engine::Cloud,
        fallback: None,
    }
}

fn replace(conn: &mut Connection, id: i64, text: &str, source: EngineUsed, now: i64) {
    HistoryManager::replace_transcription_with_conn(
        conn,
        id,
        &ProcessedText::unprocessed(text.to_string()),
        source,
        now,
    )
    .unwrap();
}

fn current(conn: &Connection, id: i64) -> HistoryEntry {
    HistoryManager::get_entry_by_id_with_conn(conn, id)
        .unwrap()
        .unwrap()
}

fn texts(conn: &Connection, id: i64) -> Vec<String> {
    HistoryManager::versions_with_conn(conn, id)
        .unwrap()
        .into_iter()
        .map(|version| version.transcription_text)
        .collect()
}

#[test]
fn a_replaced_transcript_stays_as_a_version_with_its_engine_and_time() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "first");

    replace(&mut conn, id, "second", on_this_computer(), 200);

    let entry = current(&conn, id);
    assert_eq!(entry.transcription_text, "second");
    assert_eq!(entry.engine, Engine::Local);
    assert_eq!(entry.version_count, 1);
    let versions = HistoryManager::versions_with_conn(&conn, id).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].transcription_text, "first");
    assert_eq!(versions[0].engine, Engine::Cloud);
    assert_eq!(versions[0].produced_at, 100);
}

#[test]
fn the_time_of_a_version_is_when_that_text_was_produced() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "first");

    replace(&mut conn, id, "second", cloud(), 200);
    replace(&mut conn, id, "third", cloud(), 300);

    let produced: Vec<i64> = HistoryManager::versions_with_conn(&conn, id)
        .unwrap()
        .iter()
        .map(|version| version.produced_at)
        .collect();
    assert_eq!(produced, [200, 100]);
    assert_eq!(texts(&conn, id), ["second", "first"]);
}

#[test]
fn the_raw_text_and_cleanup_of_a_replaced_transcript_come_with_it() {
    let (_directory, mut conn) = history_db();
    let cleaned = ProcessedText {
        text: "Clean.".to_string(),
        raw: "um clean".to_string(),
        cleanup: Some(CleanupOutcome::Cleaned),
    };
    let id = insert_row(&conn, "a.wav", 5, "t", &cleaned, cloud()).unwrap();

    replace(&mut conn, id, "plain", cloud(), 10);

    let version = &HistoryManager::versions_with_conn(&conn, id).unwrap()[0];
    assert_eq!(version.raw_text.as_deref(), Some("um clean"));
    assert_eq!(version.cleanup, Some(CleanupOutcome::Cleaned));
    let entry = current(&conn, id);
    assert_eq!(entry.raw_text, None);
    assert_eq!(entry.cleanup, None);
}

#[test]
fn only_the_five_newest_versions_are_kept() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "v0");
    for n in 1..=8 {
        replace(&mut conn, id, &format!("v{n}"), cloud(), 100 + n);
    }

    assert_eq!(MAX_VERSIONS, 5);
    assert_eq!(current(&conn, id).transcription_text, "v8");
    assert_eq!(texts(&conn, id), ["v7", "v6", "v5", "v4", "v3"]);
    assert_eq!(current(&conn, id).version_count, 5);
}

#[test]
fn the_text_just_replaced_stays_even_when_it_is_older_than_every_version_kept() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "old");
    for n in 1..=5 {
        replace(&mut conn, id, &format!("v{n}"), cloud(), 200 + n);
    }
    // Bring the oldest text back: the current text now dates from 100 while
    // the five versions kept all date from later.
    let oldest = HistoryManager::versions_with_conn(&conn, id).unwrap()[4].id;
    HistoryManager::restore_version_with_conn(&mut conn, id, oldest).unwrap();
    assert_eq!(current(&conn, id).transcription_text, "old");

    replace(&mut conn, id, "newest", cloud(), 900);

    let kept = texts(&conn, id);
    assert_eq!(kept.len(), MAX_VERSIONS);
    assert!(kept.contains(&"old".to_string()), "{kept:?}");
    assert_eq!(current(&conn, id).transcription_text, "newest");
}

#[test]
fn the_cap_is_per_dictation() {
    let (_directory, mut conn) = history_db();
    let busy = dictate(&conn, 100, "a0");
    let quiet = dictate(&conn, 101, "b0");
    replace(&mut conn, quiet, "b1", cloud(), 150);
    for n in 1..=7 {
        replace(&mut conn, busy, &format!("a{n}"), cloud(), 200 + n);
    }

    assert_eq!(texts(&conn, quiet), ["b0"]);
    assert_eq!(texts(&conn, busy).len(), MAX_VERSIONS);
}

#[test]
fn an_empty_first_attempt_or_an_identical_result_makes_no_version() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "");

    replace(&mut conn, id, "first words", cloud(), 200);
    assert!(texts(&conn, id).is_empty());

    replace(&mut conn, id, "first words", on_this_computer(), 300);
    assert!(texts(&conn, id).is_empty());
    assert_eq!(current(&conn, id).engine, Engine::Local);
}

#[test]
fn replacing_an_unknown_entry_fails_and_stores_nothing() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "first");

    let result = HistoryManager::replace_transcription_with_conn(
        &mut conn,
        id + 1,
        &ProcessedText::unprocessed("x".to_string()),
        cloud(),
        5,
    );

    assert!(result.is_err());
    assert!(texts(&conn, id + 1).is_empty());
    assert_eq!(current(&conn, id).transcription_text, "first");
}

#[test]
fn restoring_swaps_the_current_transcript_with_the_version() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "first");
    replace(&mut conn, id, "second", on_this_computer(), 200);
    let version_id = HistoryManager::versions_with_conn(&conn, id).unwrap()[0].id;

    HistoryManager::restore_version_with_conn(&mut conn, id, version_id).unwrap();

    let entry = current(&conn, id);
    assert_eq!(entry.transcription_text, "first");
    assert_eq!(entry.engine, Engine::Cloud);
    assert_eq!(entry.version_count, 1);
    let versions = HistoryManager::versions_with_conn(&conn, id).unwrap();
    assert_eq!(versions[0].transcription_text, "second");
    assert_eq!(versions[0].engine, Engine::Local);
    assert_eq!(versions[0].produced_at, 200);

    // Restoring again goes back; nothing was ever lost.
    HistoryManager::restore_version_with_conn(&mut conn, id, versions[0].id).unwrap();
    assert_eq!(current(&conn, id).transcription_text, "second");
    assert_eq!(texts(&conn, id), ["first"]);
}

#[test]
fn restoring_never_grows_the_list_and_refuses_another_entrys_version() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "a0");
    let other = dictate(&conn, 101, "b0");
    for n in 1..=7 {
        replace(&mut conn, id, &format!("a{n}"), cloud(), 200 + n);
    }
    replace(&mut conn, other, "b1", cloud(), 300);
    let foreign = HistoryManager::versions_with_conn(&conn, other).unwrap()[0].id;
    let oldest = HistoryManager::versions_with_conn(&conn, id).unwrap()[4].id;

    assert!(HistoryManager::restore_version_with_conn(&mut conn, id, foreign).is_err());
    assert!(HistoryManager::restore_version_with_conn(&mut conn, id, 9999).is_err());
    assert_eq!(current(&conn, id).transcription_text, "a7");

    HistoryManager::restore_version_with_conn(&mut conn, id, oldest).unwrap();
    assert_eq!(current(&conn, id).transcription_text, "a2");
    assert_eq!(texts(&conn, id).len(), MAX_VERSIONS);
    assert_eq!(texts(&conn, other), ["b0"]);
}

#[test]
fn search_follows_the_current_text_only() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "pineapple");
    replace(&mut conn, id, "mango", cloud(), 200);

    assert_eq!(found(&conn, "mango"), [id]);
    assert!(found(&conn, "pineapple").is_empty());

    let version_id = HistoryManager::versions_with_conn(&conn, id).unwrap()[0].id;
    HistoryManager::restore_version_with_conn(&mut conn, id, version_id).unwrap();
    assert_eq!(found(&conn, "pineapple"), [id]);
    assert!(found(&conn, "mango").is_empty());
}

#[test]
fn deleting_an_entry_takes_its_versions_with_it() {
    let (_directory, mut conn) = history_db();
    let id = dictate(&conn, 100, "first");
    let kept = dictate(&conn, 101, "other");
    replace(&mut conn, id, "second", cloud(), 200);
    replace(&mut conn, kept, "other 2", cloud(), 201);

    conn.execute(
        "DELETE FROM transcription_history WHERE id = ?1",
        params![id],
    )
    .unwrap();

    assert!(texts(&conn, id).is_empty());
    assert_eq!(texts(&conn, kept), ["other"]);
    let directory = tempfile::tempdir().unwrap();
    HistoryManager::clear_with_conn(&mut conn, directory.path()).unwrap();
    assert!(texts(&conn, kept).is_empty());
}

#[test]
fn a_database_from_before_versions_upgrades_in_place() {
    let mut conn = Connection::open_in_memory().unwrap();
    let before_versions = &MIGRATIONS[..MIGRATIONS.len() - 1];
    Migrations::new(before_versions.to_vec())
        .to_latest(&mut conn)
        .unwrap();
    conn.execute_batch(
        "INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text, engine
        ) VALUES ('a.wav', 100, 1, 'Old', 'kept words', 'cloud');",
    )
    .unwrap();
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);

    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut conn)
        .unwrap();

    let entry = current(&conn, 1);
    assert_eq!(entry.transcription_text, "kept words");
    assert!(entry.saved);
    assert_eq!(entry.version_count, 0);
    assert_eq!(found(&conn, "kept"), [1]);

    // The first re-transcription of an old row dates its version from the
    // dictation itself, since no transcript time was stored then.
    replace(&mut conn, 1, "new words", on_this_computer(), 500);
    let versions = HistoryManager::versions_with_conn(&conn, 1).unwrap();
    assert_eq!(versions[0].produced_at, 100);
    assert_eq!(versions[0].transcription_text, "kept words");
}
