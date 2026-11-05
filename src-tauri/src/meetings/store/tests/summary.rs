use super::*;

#[test]
fn the_summary_names_the_meeting_unless_the_user_did() {
    let (_dir, store) = store();
    let id = ready(&store);
    let fresh = store.get(id).unwrap().unwrap();
    assert_eq!(fresh.summary_status, SummaryStatus::Pending);
    assert_eq!(fresh.summary, None);

    assert!(store
        .save_summary(id, Some("Budget review"), "Notes about [[me]].")
        .unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.title.as_deref(), Some("Budget review"));
    assert_eq!(meeting.summary.as_deref(), Some("Notes about [[me]]."));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    // Written once: a second summary is not waited for.
    assert!(!store.save_summary(id, Some("Other"), "Other.").unwrap());
    assert_eq!(
        store.get(id).unwrap().unwrap().summary.as_deref(),
        Some("Notes about [[me]].")
    );

    let named = ready(&store);
    store.rename(named, Some("My title")).unwrap();
    assert!(store
        .save_summary(named, Some("Other"), "New notes.")
        .unwrap());
    let meeting = store.get(named).unwrap().unwrap();
    assert_eq!(meeting.title.as_deref(), Some("My title"));
    assert_eq!(meeting.summary.as_deref(), Some("New notes."));

    let untitled = ready(&store);
    assert!(store.save_summary(untitled, None, "Notes.").unwrap());
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
        .save_summary(id, Some("Old title"), "Old notes.")
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
        .save_summary(id, Some("New title"), "New notes.")
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
    store.save_summary(id, None, "Notes by [[s1]].").unwrap();
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
