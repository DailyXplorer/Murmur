use super::*;

#[test]
fn a_chunk_plan_is_stored_once_and_keeps_its_results() {
    let (_dir, store) = store();
    let id = recorded(&store);
    store
        .plan_chunks(
            id,
            Track::Mic,
            &plan(&[(0, 600_000, 90_000), (600_000, 700_000, 0)]),
        )
        .unwrap();
    store
        .plan_chunks(id, Track::System, &plan(&[(0, 700_000, 5_000)]))
        .unwrap();
    store.chunk_done(id, Track::Mic, 0, "Bonjour.").unwrap();
    store.chunk_failed(id, Track::System, 0, "network").unwrap();
    // Planning again after a restart changes nothing.
    store
        .plan_chunks(id, Track::Mic, &plan(&[(0, 700_000, 90_000)]))
        .unwrap();

    let chunks = store.chunks(id).unwrap();
    let summary: Vec<_> = chunks
        .iter()
        .map(|c| (c.track, c.idx, c.start_ms, c.status, c.text.as_deref()))
        .collect();
    assert_eq!(
        summary,
        [
            (Track::System, 0, 0, ChunkStatus::Failed, None),
            (Track::Mic, 0, 0, ChunkStatus::Done, Some("Bonjour.")),
            (Track::Mic, 1, 600_000, ChunkStatus::Silent, None),
        ]
    );
}

/// A run of a deleted meeting must not write into a meeting that is
/// still recording.
#[test]
fn only_a_meeting_being_processed_gets_a_plan_or_a_silence_flag() {
    let (_dir, store) = store();
    let live = store.insert_recording(0, MeetingTracks::Both).unwrap();
    store
        .plan_chunks(live, Track::Mic, &plan(&[(0, 60_000, 30_000)]))
        .unwrap();
    store.set_system_audio_silent(live, true).unwrap();
    assert!(store.chunks(live).unwrap().is_empty());
    assert!(!store.get(live).unwrap().unwrap().system_audio_silent);
}

#[test]
fn a_saved_transcript_is_read_back_in_order_and_keeps_renamed_speakers() {
    let (_dir, store) = store();
    let id = recorded(&store);
    let speakers = vec![
        MeetingSpeaker {
            key: "me".into(),
            name: "Me".into(),
            default_name: "Me".into(),
            color_index: 0,
        },
        MeetingSpeaker {
            key: "s1".into(),
            name: "Speaker 2".into(),
            default_name: "Speaker 2".into(),
            color_index: 1,
        },
    ];
    let turn = |key: &str, start_ms: i64, text: &str| MeetingTurn {
        speaker_key: key.into(),
        start_ms,
        end_ms: start_ms + 1_000,
        text: text.into(),
        low_confidence: false,
    };
    store
        .save_transcript(
            id,
            &speakers,
            &[turn("s1", 0, "Salut."), turn("me", 900, "Hello.")],
            SpeakerAnalysis::NoModels,
            None,
            None,
        )
        .unwrap();
    store.rename_speaker(id, "s1", "Sam").unwrap();
    store.rename_speaker(id, "me", "Host").unwrap();
    assert!(store.rename_speaker(id, "s9", "Nobody").is_err());

    // Processing again from the same speakers replaces the turns but
    // keeps the renames.
    store
        .save_transcript(
            id,
            &speakers,
            &[turn("s1", 0, "Salut tout le monde.")],
            SpeakerAnalysis::Failed,
            None,
            Some(0.9),
        )
        .unwrap();
    let detail = store.get(id).unwrap().unwrap();
    assert_eq!(detail.speakers[1].name, "Sam");
    assert_eq!(detail.speakers[1].default_name, "Speaker 2");
    assert_eq!(detail.turns, [turn("s1", 0, "Salut tout le monde.")]);
    assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Failed));

    // A new diarization may give "s1" to someone else: only the user
    // keeps their name.
    store
        .save_transcript(
            id,
            &speakers,
            &[turn("s1", 0, "Salut.")],
            SpeakerAnalysis::Separated,
            Some("first"),
            None,
        )
        .unwrap();
    let detail = store.get(id).unwrap().unwrap();
    assert_eq!(detail.speakers[0].name, "Host");
    assert_eq!(detail.speakers[1].name, "Speaker 2");
    assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Separated));

    // A name the user never changed follows a new default.
    let renumbered = [
        speakers[0].clone(),
        MeetingSpeaker {
            name: "Speaker 3".into(),
            default_name: "Speaker 3".into(),
            ..speakers[1].clone()
        },
        MeetingSpeaker {
            key: "s2".into(),
            name: "Speaker 2".into(),
            default_name: "Speaker 2".into(),
            color_index: 2,
        },
    ];
    store
        .save_transcript(
            id,
            &renumbered,
            &[turn("s1", 0, "Salut.")],
            SpeakerAnalysis::Separated,
            Some("first"),
            None,
        )
        .unwrap();
    let detail = store.get(id).unwrap().unwrap();
    assert_eq!(detail.speakers[1].name, "Speaker 3");
    assert_eq!(detail.speakers[2].name, "Speaker 2");

    // A speaker the new analysis no longer finds is gone.
    store
        .save_transcript(
            id,
            &speakers[..1],
            &[turn("me", 0, "Seul.")],
            SpeakerAnalysis::Failed,
            None,
            None,
        )
        .unwrap();
    let detail = store.get(id).unwrap().unwrap();
    assert_eq!(detail.speakers.len(), 1);
    assert_eq!(detail.speaker_analysis, Some(SpeakerAnalysis::Failed));
}
