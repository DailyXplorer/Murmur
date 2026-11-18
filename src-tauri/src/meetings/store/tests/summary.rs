use super::*;

#[test]
fn the_summary_names_the_meeting_unless_the_user_did() {
    let (_dir, store) = store();
    let id = ready(&store);
    let fresh = store.get(id).unwrap().unwrap();
    assert_eq!(fresh.summary_status, SummaryStatus::Pending);
    assert_eq!(fresh.summary, None);

    assert!(store
        .save_summary(id, Some("Budget review"), "Notes about [[me]].", &[])
        .unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.title.as_deref(), Some("Budget review"));
    assert_eq!(meeting.summary.as_deref(), Some("Notes about [[me]]."));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    // Written once: a second summary is not waited for.
    assert!(!store
        .save_summary(id, Some("Other"), "Other.", &[])
        .unwrap());
    assert_eq!(
        store.get(id).unwrap().unwrap().summary.as_deref(),
        Some("Notes about [[me]].")
    );

    let named = ready(&store);
    store.rename(named, Some("My title")).unwrap();
    assert!(store
        .save_summary(named, Some("Other"), "New notes.", &[])
        .unwrap());
    let meeting = store.get(named).unwrap().unwrap();
    assert_eq!(meeting.title.as_deref(), Some("My title"));
    assert_eq!(meeting.summary.as_deref(), Some("New notes."));

    let untitled = ready(&store);
    assert!(store.save_summary(untitled, None, "Notes.", &[]).unwrap());
    assert_eq!(store.get(untitled).unwrap().unwrap().title, None);
}

#[test]
fn a_failed_summary_waits_for_retry_and_then_for_processing() {
    let (_dir, store) = store();
    let id = ready(&store);
    store.audio_compressed(id, 1_000).unwrap();
    // Pending: processing writes it, at the next launch too.
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);
    assert!(!store.retry_summary(id).unwrap());

    store.summary_failed(id, SummaryError::RateLimited).unwrap();
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Failed);
    assert_eq!(meeting.summary_error, Some(SummaryError::RateLimited));
    assert!(store.waiting_for_processing().unwrap().is_empty());

    assert!(store.retry_summary(id).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.summary_error, None);
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);

    // A run that ends Failed keeps the failed summary, which only the
    // meeting's own retry can bring back.
    store.summary_failed(id, SummaryError::RateLimited).unwrap();
    assert!(store.reopen(id).unwrap());
    store
        .finish_processing(id, Some(ProcessingError::NoAudio))
        .unwrap();
    assert!(!store.retry_summary(id).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Failed);
}

#[test]
fn a_summary_of_the_old_transcript_is_dropped_after_transcribing_again() {
    let (_dir, store) = store();
    let id = ready(&store);
    // Codex is still writing the summary when the user transcribes again.
    assert!(store.retranscribe(id).unwrap());
    assert!(!store
        .save_summary(id, Some("Old title"), "Old notes.", &[])
        .unwrap());
    assert!(!store.summary_failed(id, SummaryError::Timeout).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary, None);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.summary_error, None);
    assert_eq!(meeting.title, None);

    // The new transcript gets its own summary.
    store.finish_processing(id, None).unwrap();
    assert!(store
        .save_summary(id, Some("New title"), "New notes.", &[])
        .unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary.as_deref(), Some("New notes."));
    assert_eq!(meeting.title.as_deref(), Some("New title"));
}

#[test]
fn new_speaker_keys_make_the_summary_be_written_again() {
    let (_dir, store) = store();
    let id = recorded(&store);
    let speakers = [MeetingSpeaker {
        key: "s1".into(),
        name: "Speaker 1".into(),
        default_name: "Speaker 1".into(),
        color_index: 1,
    }];
    let save = |identity: Option<&str>| {
        store
            .save_transcript(
                id,
                &speakers,
                &[],
                SpeakerAnalysis::Separated,
                identity,
                None,
            )
            .unwrap()
    };
    save(None);
    store.finish_processing(id, None).unwrap();
    store
        .save_summary(id, None, "Notes by [[s1]].", &[])
        .unwrap();
    // The same keys keep the summary.
    assert!(store.reopen(id).unwrap());
    save(None);
    assert_eq!(
        store.get(id).unwrap().unwrap().summary_status,
        SummaryStatus::Ready
    );
    save(Some("diarized"));
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.summary, None);
}

/// A transcribed meeting that waits for its summary, with the user and
/// `remote` other speakers under their default names.
fn waiting_with_speakers(store: &MeetingsStore, remote: u32) -> i64 {
    let id = recorded(store);
    let mut speakers = vec![me()];
    speakers.extend((1..=remote).map(|index| MeetingSpeaker {
        key: format!("s{index}"),
        name: format!("Speaker {}", index + 1),
        default_name: format!("Speaker {}", index + 1),
        color_index: index,
    }));
    store
        .save_transcript(
            id,
            &speakers,
            &said(&["Bonjour."]),
            SpeakerAnalysis::Separated,
            None,
            None,
        )
        .unwrap();
    store.finish_processing(id, None).unwrap();
    id
}

fn named(key: &str, name: &str) -> crate::meetings::summary::Named {
    crate::meetings::summary::Named {
        key: key.into(),
        name: name.into(),
    }
}

fn shown(store: &MeetingsStore, id: i64) -> Vec<(String, String)> {
    store
        .get(id)
        .unwrap()
        .unwrap()
        .speakers
        .into_iter()
        .map(|speaker| (speaker.key, speaker.name))
        .collect()
}

fn pair(key: &str, name: &str) -> (String, String) {
    (key.to_string(), name.to_string())
}

#[test]
fn speakers_take_the_names_said_in_the_meeting_unless_the_user_named_them() {
    let (_dir, store) = store();
    let id = waiting_with_speakers(&store, 2);
    store.rename_speaker(id, "s2", "Camille").unwrap();

    let names = [
        named("me", "Louis"),
        named("s1", "Marie"),
        named("s2", "Paul"),
        named("s9", "Nobody"),
    ];
    assert!(store.save_summary(id, None, "Notes.", &names).unwrap());

    assert_eq!(
        shown(&store, id),
        [pair("me", "Me"), pair("s1", "Marie"), pair("s2", "Camille")]
    );
    // The name is a rename like any other: the default one is kept to go
    // back to.
    assert_eq!(
        store.get(id).unwrap().unwrap().speakers[1].default_name,
        "Speaker 2"
    );
}

#[test]
fn a_name_another_speaker_shows_by_then_is_not_given() {
    let (_dir, store) = store();
    let id = waiting_with_speakers(&store, 3);
    // Renamed while the summary was being written.
    store.rename_speaker(id, "s2", "Marie").unwrap();

    let names = [
        named("s1", "marie"),
        // The default name of another speaker.
        named("s3", "Speaker 2"),
    ];
    assert!(store.save_summary(id, None, "Notes.", &names).unwrap());

    assert_eq!(
        shown(&store, id),
        [
            pair("me", "Me"),
            pair("s1", "Speaker 2"),
            pair("s2", "Marie"),
            pair("s3", "Speaker 4"),
        ]
    );

    // Within one summary too: the second speaker given a name keeps its own.
    let other = waiting_with_speakers(&store, 2);
    let names = [named("s1", "Paul"), named("s2", "PAUL")];
    assert!(store.save_summary(other, None, "Notes.", &names).unwrap());
    assert_eq!(
        shown(&store, other),
        [
            pair("me", "Me"),
            pair("s1", "Paul"),
            pair("s2", "Speaker 3")
        ]
    );

    // A capital with an accent is the same letter.
    let accented = waiting_with_speakers(&store, 2);
    store.rename_speaker(accented, "s2", "Élodie").unwrap();
    assert!(store
        .save_summary(accented, None, "Notes.", &[named("s1", "élodie")])
        .unwrap());
    assert_eq!(
        shown(&store, accented),
        [
            pair("me", "Me"),
            pair("s1", "Speaker 2"),
            pair("s2", "Élodie")
        ]
    );
}

#[test]
fn a_summary_no_longer_waited_for_names_nobody() {
    let (_dir, store) = store();
    let id = waiting_with_speakers(&store, 1);
    assert!(store.save_summary(id, None, "Notes.", &[]).unwrap());

    assert!(!store
        .save_summary(id, None, "Other notes.", &[named("s1", "Marie")])
        .unwrap());

    assert_eq!(
        shown(&store, id),
        [pair("me", "Me"), pair("s1", "Speaker 2")]
    );
}

#[test]
fn the_summary_and_its_names_are_saved_together_or_not_at_all() {
    let (_dir, store) = store();
    let id = waiting_with_speakers(&store, 1);
    store
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_names BEFORE UPDATE ON meeting_speakers
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();

    assert!(store
        .save_summary(id, Some("Budget"), "Notes.", &[named("s1", "Marie")])
        .is_err());
    // Still waited for: the next attempt writes the summary and the names.
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.summary, None);
    assert_eq!(meeting.title, None);

    store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_names;")
        .unwrap();
    assert!(store
        .save_summary(id, Some("Budget"), "Notes.", &[named("s1", "Marie")])
        .unwrap());
    assert_eq!(shown(&store, id), [pair("me", "Me"), pair("s1", "Marie")]);
}
