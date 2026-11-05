use super::*;

#[test]
fn migrations_are_valid() {
    migrations().validate().unwrap();
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
    conn.execute_batch(
        "DROP TRIGGER meeting_turns_fts_insert;
         DROP TRIGGER meeting_turns_fts_delete;
         DROP TRIGGER meeting_turns_fts_update;
         DROP TRIGGER meetings_fts_insert;
         DROP TRIGGER meetings_fts_delete;
         DROP TRIGGER meetings_fts_update;
         DROP TABLE meeting_turns_fts;
         DROP TABLE meetings_fts;
         DROP TABLE meeting_last_id;
         ALTER TABLE meetings DROP COLUMN dropped_ms;
         ALTER TABLE meetings DROP COLUMN compress_failures;
         ALTER TABLE meetings DROP COLUMN slept_ms;
         ALTER TABLE meetings DROP COLUMN previous_id;
         ALTER TABLE meetings DROP COLUMN part;
         ALTER TABLE meetings DROP COLUMN output_route;
         PRAGMA user_version = 6;",
    )
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

#[test]
fn migration_indexes_existing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meetings.db");
    let mut conn = Connection::open(&path).unwrap();
    // The library as it was before search.
    migrations().to_version(&mut conn, 13).unwrap();
    conn.execute_batch(
        "INSERT INTO meetings (id, started_at_ms, capture_mode, status, title, summary_md)
         VALUES (7, 1000, 'mic', 'ready', 'Quarterly review', 'Numbers.');
         INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text)
         VALUES (7, 0, 'me', 0, 10, 'The forecast looks fine.');",
    )
    .unwrap();
    drop(conn);

    let (store, startup) = MeetingsStore::open(path, now());

    assert_eq!(startup, DatabaseStartup::Ready);
    assert_eq!(found(&store, "quarterly"), [7]);
    assert_eq!(found(&store, "forecast"), [7]);
}
