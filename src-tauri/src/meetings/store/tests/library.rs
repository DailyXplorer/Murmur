use super::*;

#[test]
fn a_recording_is_listed_newest_first_once_finished() {
    let (_dir, store) = store();
    let first = store.insert_recording(1_000, MeetingTracks::Both).unwrap();
    let second = store.insert_recording(5_000, MeetingTracks::Mic).unwrap();
    store.finish_recording(first, 4_000, 3_000).unwrap();

    let page = store.list(None, 10).unwrap();
    assert!(!page.has_more);
    assert_eq!(
        page.meetings,
        vec![
            MeetingListItem {
                id: second,
                title: None,
                started_at_ms: 5_000,
                duration_ms: 0,
                status: MeetingStatus::Recording,
                tracks: MeetingTracks::Mic,
            },
            MeetingListItem {
                id: first,
                title: None,
                started_at_ms: 1_000,
                duration_ms: 3_000,
                status: MeetingStatus::Processing,
                tracks: MeetingTracks::Both,
            },
        ]
    );
    assert!(store.finish_recording(first, 9_000, 8_000).is_err());
}

#[test]
fn pages_continue_after_the_cursor() {
    let (_dir, store) = store();
    let ids: Vec<i64> = (0..5)
        .map(|index| store.insert_recording(index, MeetingTracks::Both).unwrap())
        .collect();

    let first = store.list(None, 2).unwrap();
    assert!(first.has_more);
    assert_eq!(
        first.meetings.iter().map(|m| m.id).collect::<Vec<_>>(),
        [ids[4], ids[3]]
    );
    let last = store.list(Some(ids[1]), 2).unwrap();
    assert!(!last.has_more);
    assert_eq!(
        last.meetings.iter().map(|m| m.id).collect::<Vec<_>>(),
        [ids[0]]
    );
}

#[test]
fn tracks_can_be_narrowed_after_a_failed_source() {
    let (_dir, store) = store();
    let id = store.insert_recording(0, MeetingTracks::Both).unwrap();
    store.set_tracks(id, MeetingTracks::Mic).unwrap();
    assert_eq!(
        store.list(None, 1).unwrap().meetings[0].tracks,
        MeetingTracks::Mic
    );
}

#[test]
fn deleting_a_meeting_cascades_to_its_rows() {
    let (_dir, store) = store();
    let id = store.insert_recording(0, MeetingTracks::Both).unwrap();
    let conn = store.connection().unwrap();
    conn.execute(
        "INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text)
         VALUES (?1, 0, 'me', 0, 10, 'hello')",
        params![id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO meeting_speakers (meeting_id, key, default_name, display_name, color_index)
         VALUES (?1, 'me', 'Me', 'Me', 0)",
        params![id],
    )
    .unwrap();

    store.delete(id).unwrap();

    for table in ["meetings", "meeting_turns", "meeting_speakers"] {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}

/// The run of a deleted meeting may still be saving its transcript: the
/// next meeting must not take its id, nor its folder.
#[test]
fn a_deleted_meeting_id_is_never_given_again() {
    let (_dir, store) = store();
    let kept = recorded(&store);
    let deleted = recorded(&store);
    store.delete(deleted).unwrap();
    let next = store.insert_recording(0, MeetingTracks::Both).unwrap();
    assert!(next > deleted && kept < deleted);
}

#[test]
fn a_meeting_can_be_renamed_and_given_back_its_default_name() {
    let (_dir, store) = store();
    let id = recorded(&store);
    store.rename(id, Some("Point hebdo")).unwrap();
    assert_eq!(
        store.get(id).unwrap().unwrap().title.as_deref(),
        Some("Point hebdo")
    );
    assert_eq!(
        store.list(None, 1).unwrap().meetings[0].title.as_deref(),
        Some("Point hebdo")
    );
    store.rename(id, None).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().title, None);
    assert!(store.rename(999, Some("x")).is_err());
    assert_eq!(store.get(999).unwrap(), None);
}

#[test]
fn the_audio_a_recording_dropped_is_kept_with_the_meeting() {
    let (_dir, store) = store();
    let id = recorded(&store);
    assert_eq!(store.dropped_ms(id).unwrap(), 0);
    store.set_dropped_ms(id, 1_250).unwrap();
    assert_eq!(store.dropped_ms(id).unwrap(), 1_250);
}

#[test]
fn a_meeting_keeps_where_it_played_sound() {
    let (_dir, store) = store();
    let id = recorded(&store);
    assert_eq!(store.output_route(id).unwrap(), OutputRoute::Unknown);
    store.set_output_route(id, OutputRoute::Headphones).unwrap();
    assert_eq!(store.output_route(id).unwrap(), OutputRoute::Headphones);
}

#[test]
fn each_sleep_during_a_recording_adds_to_the_time_asleep() {
    let (_dir, store) = store();
    let id = recorded(&store);
    assert_eq!(store.slept_ms(id).unwrap(), 0);
    store.add_slept_ms(id, 1_200_000).unwrap();
    store.add_slept_ms(id, 30_000).unwrap();
    assert_eq!(store.slept_ms(id).unwrap(), 1_230_000);
}

#[test]
fn a_row_marked_as_a_later_part_reads_as_a_meeting_of_its_own() {
    let (_dir, store) = store();
    let first = recorded(&store);
    let second = recorded(&store);
    store
        .connection()
        .unwrap()
        .execute(
            "UPDATE meetings SET previous_id = ?1, part = 2 WHERE id = ?2",
            params![first, second],
        )
        .unwrap();

    for id in [first, second] {
        let detail = serde_json::to_value(store.get(id).unwrap().unwrap()).unwrap();
        for key in ["part", "previous_id", "next_id"] {
            assert!(detail.get(key).is_none(), "meeting {id} has {key}");
        }
    }
    for item in store.list(None, 10).unwrap().meetings {
        let item = serde_json::to_value(item).unwrap();
        assert!(item.get("part").is_none(), "{item}");
    }
}
