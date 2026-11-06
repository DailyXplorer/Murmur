use super::*;

#[test]
fn a_lost_session_fails_the_meeting_with_sign_in_needed() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Mic, 20);
    track(
        &paths::meeting_dir(&h.root, id).join(paths::MIC_TRACK),
        20,
        &[(0, 20)],
    );
    h.process(id, codex(|_| Err(FailureKind::SessionExpired)));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.error, Some(ProcessingError::SignedOut));
    assert_eq!(
        *h.events.1.lock().unwrap(),
        [Finished {
            id,
            outcome: FinishedOutcome::Failed
        }]
    );
}

#[test]
fn ready_is_announced_before_the_summary_and_only_once() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![]);
    let events = Arc::clone(&h.events);
    let store = Arc::clone(&h.store);
    *summarizer.meanwhile.lock().unwrap() = Some(Box::new(move || {
        assert_eq!(
            *events.1.lock().unwrap(),
            [Finished {
                id,
                outcome: FinishedOutcome::Ready
            }]
        );
        assert_eq!(store.get(id).unwrap().unwrap().status, MeetingStatus::Ready);
    }));
    h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        summarizer,
    );
    h.process(
        id,
        codex(|_| panic!("A ready meeting must not be transcribed again")),
    );
    assert_eq!(
        *h.events.1.lock().unwrap(),
        [Finished {
            id,
            outcome: FinishedOutcome::Ready
        }]
    );
}

#[test]
fn every_terminal_processing_error_announces_a_failure_once() {
    let h = harness();
    let processor = Processor {
        store: Arc::clone(&h.store),
        root: h.root.clone(),
        transcriber: codex(|_| Ok(String::new())),
        local: None,
        analyzer: Arc::new(FakeAnalyzer::without_models()),
        summarizer: FakeSummarizer::answering(vec![]),
        policy: RetryPolicy::default(),
        language: None,
        events: Arc::clone(&h.events) as Arc<dyn ProcessEvents>,
        compress_audio: false,
    };
    let mut expected = Vec::new();
    for error in [
        ProcessingError::SignedOut,
        ProcessingError::NoAudio,
        ProcessingError::RateLimited,
        ProcessingError::Offline,
        ProcessingError::PartsFailed,
        ProcessingError::Other,
    ] {
        let id = h.meeting(MeetingTracks::Mic, 20);
        h.store.begin_processing(id).unwrap();
        processor.finish_processing(id, Some(error)).unwrap();
        processor.finish_processing(id, Some(error)).unwrap();
        expected.push(Finished {
            id,
            outcome: FinishedOutcome::Failed,
        });
        assert_eq!(h.store.get(id).unwrap().unwrap().error, Some(error));
        assert_eq!(*h.events.1.lock().unwrap(), expected);
    }
}

#[test]
fn a_meeting_without_readable_audio_fails_and_a_ready_one_is_left_alone() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 20);
    let codex = codex(|_| Ok("x".into()));
    h.process(id, Arc::clone(&codex));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::NoAudio));

    let ready = h.meeting(MeetingTracks::Mic, 20);
    h.store.begin_processing(ready).unwrap();
    h.store.finish_processing(ready, None).unwrap();
    h.process(ready, Arc::clone(&codex));
    assert!(codex.calls.lock().unwrap().is_empty());
    assert_eq!(
        *h.events.1.lock().unwrap(),
        [Finished {
            id,
            outcome: FinishedOutcome::Failed
        }]
    );
}

#[test]
fn a_recorded_track_that_cannot_be_read_fails_the_meeting_and_keeps_the_other() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 20);
    // System audio was recorded, but its file is gone.
    track(
        &paths::meeting_dir(&h.root, id).join(paths::MIC_TRACK),
        20,
        &[(2, 10)],
    );
    let summarizer = FakeSummarizer::answering(vec![]);
    h.process_all(
        id,
        codex(|_| Ok("Seul.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.error, Some(ProcessingError::NoAudio));
    assert_eq!(summarizer.calls(), 0, "no summary from half a meeting");
    assert_eq!(turns(&meeting), [("me", "Seul.")]);
}

/// Deleting a meeting cancels its run, maybe while the tracks are read;
/// nothing may be stored after.
#[test]
fn a_run_cancelled_while_reading_the_tracks_stores_nothing() {
    let h = harness();
    let id = spoken_meeting(&h);
    let codex = codex(|_| Ok("Bonjour.".into()));
    let operation = ProcessingOperation::new(OperationId(id as u64));
    assert!(operation.cancel());
    h.process_during(
        id,
        Arc::clone(&codex),
        Arc::new(FakeAnalyzer::without_models()),
        FakeSummarizer::answering(vec![]),
        operation,
    );
    assert!(h.store.chunks(id).unwrap().is_empty());
    assert!(codex.calls.lock().unwrap().is_empty());
    assert!(h.events.1.lock().unwrap().is_empty());
}

/// "Transcribe again" on audio that can no longer be read leaves the
/// meeting's transcript and summary as they were.
#[test]
fn transcribing_again_unreadable_audio_keeps_the_summary() {
    let h = harness();
    let id = spoken_meeting(&h);
    h.process(id, codex(|_| Ok("Bonjour.".into())));
    let before = h.store.get(id).unwrap().unwrap();
    assert_eq!(before.summary_status, SummaryStatus::Ready);
    let dir = paths::meeting_dir(&h.root, id);
    for file in [paths::MIC_TRACK, paths::SYSTEM_TRACK, paths::AUDIO_FILE] {
        let _ = std::fs::remove_file(dir.join(file));
    }

    assert!(h.store.retranscribe(id).unwrap());
    h.process(id, codex(|_| Ok("never".into())));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::NoAudio));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(meeting.summary, before.summary);
    assert_eq!(turns(&meeting), turns(&before));
}

#[test]
fn transcribing_again_with_one_unreadable_track_keeps_the_transcript() {
    let h = harness();
    let id = spoken_meeting(&h);
    h.process(id, codex(|_| Ok("Bonjour.".into())));
    let before = h.store.get(id).unwrap().unwrap();
    assert_eq!(before.summary_status, SummaryStatus::Ready);
    assert_eq!(before.turns.len(), 2);
    // The microphone track reads, the system one is damaged.
    let dir = paths::meeting_dir(&h.root, id);
    let _ = std::fs::remove_file(dir.join(paths::AUDIO_FILE));
    track(&dir.join(paths::MIC_TRACK), 20, &[(8, 14)]);
    std::fs::write(dir.join(paths::SYSTEM_TRACK), b"damaged").unwrap();

    assert!(h.store.retranscribe(id).unwrap());
    h.process(id, codex(|_| Ok("Autre chose.".into())));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::NoAudio));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(meeting.summary, before.summary);
    assert_eq!(turns(&meeting), turns(&before));
}

/// "Transcribe again" while Codex cannot answer: the transcript, its
/// summary and the names the user gave stay until a Retry, once Codex
/// answers, completes the new transcript.
fn transcribing_again_while_codex_fails(failure: FailureKind, error: ProcessingError) {
    let h = harness();
    let id = spoken_meeting(&h);
    h.process(id, codex(|_| Ok("Bonjour.".into())));
    h.store.rename_speaker(id, "s1", "Camille").unwrap();
    h.store.rename_speaker(id, "me", "Alex").unwrap();
    let before = h.store.get(id).unwrap().unwrap();
    assert_eq!(before.summary_status, SummaryStatus::Ready);

    assert!(h.store.retranscribe(id).unwrap());
    h.process(id, codex(move |_| Err(failure)));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.error, Some(error));
    assert_eq!(turns(&meeting), turns(&before));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(meeting.summary, before.summary);
    assert_eq!(meeting.speakers, before.speakers);

    h.process(id, codex(|_| Ok("Au revoir.".into())));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert!(meeting.turns.iter().all(|turn| turn.text == "Au revoir."));
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Alex", "Camille"]);
}

#[test]
fn transcribing_again_signed_out_keeps_the_transcript_and_the_names() {
    transcribing_again_while_codex_fails(FailureKind::SessionExpired, ProcessingError::SignedOut);
}

#[test]
fn transcribing_again_offline_keeps_the_transcript_and_the_names() {
    transcribing_again_while_codex_fails(FailureKind::Network, ProcessingError::Offline);
}

/// A transcript with a part missing stays when transcribing it again
/// leaves more of the meeting out.
#[test]
fn transcribing_again_keeps_a_partial_transcript_that_holds_more() {
    let h = harness();
    let id = spoken_meeting(&h);
    // System audio speaks at 2 s; its piece is refused.
    h.process(
        id,
        codex_by_audio(|samples| {
            if samples[2 * RATE..3 * RATE].iter().any(|s| s.abs() > 0.1) {
                Err(FailureKind::Rejected)
            } else {
                Ok("Oui.".into())
            }
        }),
    );
    h.store.rename_speaker(id, "me", "Alex").unwrap();
    let before = h.store.get(id).unwrap().unwrap();
    assert_eq!(before.error, Some(ProcessingError::PartsFailed));
    assert!(turns(&before).contains(&("me", "Oui.")));

    assert!(h.store.retranscribe(id).unwrap());
    h.process(id, codex(|_| Err(FailureKind::Rejected)));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::PartsFailed));
    assert_eq!(turns(&meeting), turns(&before));
    assert_eq!(meeting.speakers, before.speakers);
}
