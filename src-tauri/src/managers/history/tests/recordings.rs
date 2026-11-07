use super::*;

#[test]
fn asset_protocol_allows_only_literal_recording_wavs() {
    let config = include_str!("../../../../tauri.conf.json");

    assert!(config.contains("\"allow\": [\"$APPDATA/recordings/*.wav\"]"));
    assert!(config.contains("\"requireLiteralLeadingDot\": true"));
}

#[test]
fn windows_also_allows_the_recordings_in_local_app_data() {
    let overlay = include_str!("../../../../tauri.windows.conf.json");
    let overlay: serde_json::Value = serde_json::from_str(overlay).expect("parse the overlay");

    // Recordings live in Local app data, or still in Roaming while their
    // move is pending. A merge replaces a list, so the overlay holds both.
    assert_eq!(
        overlay["app"]["security"]["assetProtocol"]["scope"]["allow"],
        serde_json::json!([
            "$APPLOCALDATA/recordings/*.wav",
            "$APPDATA/recordings/*.wav"
        ])
    );
    // Nothing else of the scope is overridden.
    assert!(overlay["app"]["security"]["assetProtocol"]["scope"]
        .get("requireLiteralLeadingDot")
        .is_none());
}

#[test]
fn resolves_existing_root_level_wav_inside_canonical_recordings_directory() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let recording = recordings_dir.join("murmur-100.wav");
    std::fs::write(&recording, "wav").expect("write recording");

    let resolved = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "murmur-100.wav")
        .expect("resolve recording");

    assert_eq!(
        resolved,
        std::fs::canonicalize(&recording).expect("canonicalize recording")
    );
    assert!(resolved.starts_with(
        std::fs::canonicalize(&recordings_dir).expect("canonicalize recordings directory")
    ));
}

#[test]
fn rejects_invalid_recording_file_names() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    std::fs::create_dir(recordings_dir.join("directory.wav")).expect("create recording directory");

    let absolute_path = temp.path().join("outside.wav");
    let invalid_names = vec![
        String::new(),
        ".".to_string(),
        "..".to_string(),
        "../outside.wav".to_string(),
        absolute_path.display().to_string(),
        "nested/recording.wav".to_string(),
        r"nested\recording.wav".to_string(),
        ".hidden.wav".to_string(),
        "recording.WAV".to_string(),
        "recording.mp3".to_string(),
        "missing.wav".to_string(),
        "directory.wav".to_string(),
    ];

    for file_name in invalid_names {
        let error = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, &file_name)
            .expect_err("reject invalid recording name");
        assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
    }
}

#[cfg(unix)]
#[test]
fn rejects_external_recording_symlinks() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let external_recording = temp.path().join("external.wav");
    std::fs::write(&external_recording, "wav").expect("write external recording");
    std::os::unix::fs::symlink(&external_recording, recordings_dir.join("linked.wav"))
        .expect("create recording symlink");

    let error = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "linked.wav")
        .expect_err("reject recording symlink");

    assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_recordings_root_for_playback_resolution() {
    let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();

    let error = HistoryManager::resolve_recording_path_in_dir(&recordings_dir, "murmur-100.wav")
        .expect_err("reject symlinked recordings root");

    assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("read external sentinel"),
        "sentinel"
    );
}

#[cfg(unix)]
#[test]
fn deletion_with_symlinked_recordings_root_keeps_external_sentinel_and_removes_database_row() {
    let (_temp, recordings_dir, sentinel) = symlinked_recordings_root_with_sentinel();
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "murmur-100.wav");

    HistoryManager::delete_entry_with_conn(&conn, &recordings_dir, id)
        .expect("delete history entry with symlinked recordings root");

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
fn rejects_missing_or_non_directory_recordings_roots() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let missing_root = temp.path().join("missing-recordings");
    let non_directory_root = temp.path().join("recordings-file");
    std::fs::write(&non_directory_root, "not a directory").expect("write recordings file");

    for recordings_dir in [&missing_root, &non_directory_root] {
        let error = HistoryManager::resolve_recording_path_in_dir(recordings_dir, "murmur-100.wav")
            .expect_err("reject invalid recordings root");
        assert_eq!(error.to_string(), RECORDING_UNAVAILABLE_ERROR);
    }
}

#[test]
fn deletion_of_invalid_recording_keeps_external_sentinel_and_removes_database_row() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let recordings_dir = temp.path().join("recordings");
    std::fs::create_dir(&recordings_dir).expect("create recordings directory");
    let sentinel = temp.path().join("sentinel.wav");
    std::fs::write(&sentinel, "sentinel").expect("write sentinel");
    let conn = setup_conn();
    let id = insert_entry_with_file_name(&conn, "../sentinel.wav");

    HistoryManager::delete_entry_with_conn(&conn, &recordings_dir, id)
        .expect("delete invalid history entry");

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
