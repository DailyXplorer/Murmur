use super::*;

#[test]
fn audio_is_compressed_then_deleted_and_the_text_stays() {
    let (_dir, store) = store();
    let recording = store.insert_recording(0, MeetingTracks::Mic).unwrap();
    assert!(!store.delete_audio(recording, || Ok(())).unwrap());

    let id = ready(&store);
    store.save_summary(id, None, "Notes.").unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().audio, AudioState::Tracks);
    // Waiting for its audio to be compressed.
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);

    assert!(store.audio_compressed(id, 4_200).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.audio, AudioState::Compressed);
    assert_eq!(meeting.audio_bytes, 4_200);
    assert!(store.waiting_for_processing().unwrap().is_empty());

    assert!(store.delete_audio(id, || Ok(())).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.audio, AudioState::Deleted);
    assert_eq!(meeting.audio_bytes, 0);
    assert_eq!(meeting.summary.as_deref(), Some("Notes."));
    assert!(!store.audio_compressed(id, 4_200).unwrap());
    assert!(store.waiting_for_processing().unwrap().is_empty());
    assert!(!store.retranscribe(id).unwrap());
}

#[test]
fn audio_deletion_reserves_the_writer_until_commit() {
    let (_dir, store) = store();
    let id = ready(&store);
    store.audio_compressed(id, 4_200).unwrap();
    let before = store.get(id).unwrap().unwrap();
    assert!(store
        .delete_audio(id, || {
            // Readers do not report deletion until filesystem cleanup
            // succeeds, but a competing state change cannot win.
            assert_eq!(store.get(id).unwrap().unwrap(), before);
            let conn = store.connection().unwrap();
            conn.busy_timeout(std::time::Duration::ZERO).unwrap();
            let error = conn
                .execute(
                    "UPDATE meetings SET status = 'processing'
                     WHERE id = ?1 AND audio_deleted = 0",
                    params![id],
                )
                .unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            Ok(())
        })
        .unwrap());
    assert!(!store.retranscribe(id).unwrap());
    assert!(!store.reopen(id).unwrap());
    assert!(!store.audio_compressed(id, 4_200).unwrap());
}

#[test]
fn audio_deletion_does_not_touch_recording_or_processing_files() {
    let (_dir, store) = store();
    let recording = store.insert_recording(0, MeetingTracks::Both).unwrap();
    let processing = recorded(&store);
    let retranscribing = ready(&store);
    assert!(store.retranscribe(retranscribing).unwrap());
    for id in [recording, processing, retranscribing, 999] {
        assert!(!store
            .delete_audio(id, || panic!("unfinished audio must not be unlinked"))
            .unwrap());
    }
    store
        .finish_processing(processing, Some(ProcessingError::NoAudio))
        .unwrap();
    assert!(store.delete_audio(processing, || Ok(())).unwrap());
}

#[test]
fn interrupted_audio_deletion_is_retried() {
    const CHILD_ROOT: &str = "MURMUR_AUDIO_DELETE_TEST_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let (store, _) = MeetingsStore::open(root.join("meetings.db"), Utc::now());
        // Exit without unwinding after one real unlink and before the
        // transaction can commit: SQLite must recover its journal.
        store
            .delete_audio(1, || {
                std::fs::remove_file(root.join("meetings/1").join(paths::AUDIO_FILE))?;
                std::process::exit(73);
            })
            .unwrap();
        panic!("child did not interrupt deletion");
    }
    let (dir, store) = store();
    let id = ready(&store);
    assert_eq!(id, 1);
    store.audio_compressed(id, 10).unwrap();
    let before = store.get(id).unwrap().unwrap();
    let root = dir.path().join("meetings");
    let meeting_dir = paths::fresh_meeting_dir(&root, id, 0).unwrap();
    std::fs::write(meeting_dir.join(paths::AUDIO_FILE), b"audio").unwrap();
    std::fs::write(meeting_dir.join(paths::MIC_TRACK), b"audio").unwrap();
    drop(store);

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "meetings::store::tests::audio::interrupted_audio_deletion_is_retried",
            "--nocapture",
        ])
        .env(CHILD_ROOT, dir.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), Utc::now());
    assert_eq!(store.get(id).unwrap().unwrap(), before);
    assert_eq!(store.with_audio().unwrap()[0].id, id);
    assert!(!meeting_dir.join(paths::AUDIO_FILE).exists());
    assert!(meeting_dir.join(paths::MIC_TRACK).exists());
    crate::meetings::delete_audio(&store, &root, id).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().audio, AudioState::Deleted);
    assert!(!meeting_dir.join(paths::MIC_TRACK).exists());
}

#[test]
fn audio_that_failed_to_compress_three_times_is_no_longer_requeued() {
    let (_dir, store) = store();
    let id = ready(&store);
    store.save_summary(id, None, "Notes.").unwrap();
    for _ in 0..2 {
        store.compress_failed(id).unwrap();
    }
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);

    store.compress_failed(id).unwrap();
    assert!(store.waiting_for_processing().unwrap().is_empty());
    // The tracks stay: the meeting plays and transcribes again.
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.audio, AudioState::Tracks);
    assert!(store.retranscribe(id).unwrap());
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);
}
