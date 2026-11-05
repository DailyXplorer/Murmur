use super::*;

#[test]
fn a_meeting_cut_off_by_a_crash_is_recovered_as_interrupted() {
    let (dir, store) = store();
    let root = dir.path().join("meetings");
    let crashed = store.insert_recording(0, MeetingTracks::Both).unwrap();
    let finished = store.insert_recording(1, MeetingTracks::Mic).unwrap();
    store.finish_recording(finished, 2, 1_000).unwrap();
    let track_dir = paths::meeting_dir(&root, crashed);
    write_track(&track_dir.join(paths::MIC_TRACK), 32_000);
    write_track(&track_dir.join(paths::SYSTEM_TRACK), 31_840);

    assert_eq!(store.recover_interrupted(&root).unwrap(), [crashed]);

    assert_eq!(status_of(&store, crashed), Some(MeetingStatus::Interrupted));
    assert_eq!(status_of(&store, finished), Some(MeetingStatus::Processing));
    let page = store.list(None, 10).unwrap();
    let recovered = page.meetings.iter().find(|m| m.id == crashed).unwrap();
    assert_eq!(recovered.duration_ms, 2_000);
    for file in [paths::MIC_TRACK, paths::SYSTEM_TRACK] {
        let reader = hound::WavReader::open(track_dir.join(file)).unwrap();
        assert!(reader.duration() >= 31_840);
    }
    assert!(store.recover_interrupted(&root).unwrap().is_empty());
}

#[test]
fn a_meeting_cut_off_by_a_crash_is_processed_at_the_next_launch() {
    let (dir, store) = store();
    let root = dir.path().join("meetings");
    let crashed = store.insert_recording(0, MeetingTracks::Mic).unwrap();
    write_track(
        &paths::meeting_dir(&root, crashed).join(paths::MIC_TRACK),
        32_000,
    );
    assert_eq!(store.recover_interrupted(&root).unwrap(), [crashed]);
    assert_eq!(store.waiting_for_processing().unwrap(), [crashed]);
    assert!(store.begin_processing(crashed).unwrap());
}

#[test]
fn a_meeting_that_waited_for_the_connection_is_processed_at_the_next_launch() {
    let (_dir, store) = store();
    let offline = recorded(&store);
    let parts_failed = recorded(&store);
    for (id, error) in [
        (offline, ProcessingError::Offline),
        (parts_failed, ProcessingError::PartsFailed),
    ] {
        store.begin_processing(id).unwrap();
        store.finish_processing(id, Some(error)).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().error, Some(error));
    }
    assert_eq!(store.waiting_for_processing().unwrap(), [offline]);
}

#[test]
fn a_retried_meeting_waits_for_processing_at_once() {
    let (dir, store) = store();
    let failed = recorded(&store);
    store.begin_processing(failed).unwrap();
    store
        .finish_processing(failed, Some(ProcessingError::SignedOut))
        .unwrap();
    let interrupted = store.insert_recording(1, MeetingTracks::Mic).unwrap();
    store.recover_interrupted(dir.path()).unwrap();

    // Another meeting is processing: the worker takes these later.
    for id in [failed, interrupted] {
        assert!(store.wait_for_retry(id).unwrap());
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Processing);
        assert_eq!(meeting.stage, None);
        assert!(!store.wait_for_retry(id).unwrap());
    }
    // A quit before their turn processes them at the next launch.
    assert_eq!(
        store.waiting_for_processing().unwrap(),
        [failed, interrupted]
    );
    assert!(store.begin_processing(failed).unwrap());
    assert_eq!(store.get(failed).unwrap().unwrap().error, None);
    store.finish_processing(failed, None).unwrap();
    assert!(!store.wait_for_retry(failed).unwrap(), "ready");
}

#[test]
fn a_meeting_cut_off_with_under_a_second_of_audio_fails_without_a_run() {
    let (dir, store) = store();
    let root = dir.path().join("meetings");
    let empty = store.insert_recording(0, MeetingTracks::System).unwrap();
    let short = store.insert_recording(1, MeetingTracks::Mic).unwrap();
    write_track(
        &paths::meeting_dir(&root, short).join(paths::MIC_TRACK),
        15_000,
    );
    assert!(store.recover_interrupted(&root).unwrap().is_empty());
    let page = store.list(None, 2).unwrap();
    for meeting in &page.meetings {
        assert_eq!(meeting.status, MeetingStatus::Failed);
    }
    assert_eq!(page.meetings[0].duration_ms, 937);
    assert_eq!(page.meetings[1].id, empty);
    assert_eq!(page.meetings[1].duration_ms, 0);
    for id in [empty, short] {
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.error, Some(ProcessingError::NoAudio));
    }
    assert!(store.waiting_for_processing().unwrap().is_empty());
}

#[test]
fn only_a_meeting_being_processed_gets_a_transcript_or_a_stage() {
    let (_dir, store) = store();
    let live = store.insert_recording(0, MeetingTracks::Mic).unwrap();
    let me = MeetingSpeaker {
        key: "me".into(),
        name: "Me".into(),
        default_name: "Me".into(),
        color_index: 0,
    };
    let turn = MeetingTurn {
        speaker_key: "me".into(),
        start_ms: 0,
        end_ms: 1_000,
        text: "Deleted.".into(),
        low_confidence: false,
    };
    store
        .save_transcript(live, &[me], &[turn], SpeakerAnalysis::Separated, None, None)
        .unwrap();
    store.set_stage(live, MeetingStage::Aligning).unwrap();
    let detail = store.get(live).unwrap().unwrap();
    assert!(detail.turns.is_empty() && detail.speakers.is_empty());
    assert_eq!((detail.stage, detail.speaker_analysis), (None, None));
}

#[test]
fn processing_runs_only_on_recorded_meetings_and_records_its_outcome() {
    let (_dir, store) = store();
    let live = store.insert_recording(0, MeetingTracks::Mic).unwrap();
    assert!(!store.begin_processing(live).unwrap());
    assert!(!store.begin_processing(999).unwrap());

    let id = recorded(&store);
    assert_eq!(store.waiting_for_processing().unwrap(), [id]);
    assert!(store.begin_processing(id).unwrap());
    store.set_stage(id, MeetingStage::Aligning).unwrap();
    assert_eq!(
        store.get(id).unwrap().unwrap().stage,
        Some(MeetingStage::Aligning)
    );
    store
        .finish_processing(id, Some(ProcessingError::SignedOut))
        .unwrap();
    let failed = store.get(id).unwrap().unwrap();
    assert_eq!(failed.status, MeetingStatus::Failed);
    assert_eq!(failed.error, Some(ProcessingError::SignedOut));
    assert_eq!(failed.stage, None);
    assert!(store.waiting_for_processing().unwrap().is_empty());

    // Retrying clears the error; success makes it ready, for good.
    assert!(store.begin_processing(id).unwrap());
    assert_eq!(store.get(id).unwrap().unwrap().error, None);
    store.finish_processing(id, None).unwrap();
    assert_eq!(store.get(id).unwrap().unwrap().status, MeetingStatus::Ready);
    assert!(!store.begin_processing(id).unwrap());
}

#[test]
fn only_a_finished_meeting_can_be_reopened() {
    let (_dir, store) = store();
    let id = recorded(&store);
    assert!(!store.reopen(id).unwrap());
    store.begin_processing(id).unwrap();
    store.finish_processing(id, None).unwrap();
    assert!(store.reopen(id).unwrap());
    assert_eq!(
        store.get(id).unwrap().unwrap().status,
        MeetingStatus::Processing
    );
    assert!(store.begin_processing(id).unwrap());
}

#[test]
fn a_meeting_whose_audio_went_is_not_reopened() {
    let (_dir, store) = store();
    let id = recorded(&store);
    store.begin_processing(id).unwrap();
    store.finish_processing(id, None).unwrap();
    // The daily retention sweep, between Reanalyze's check and `reopen`.
    assert!(store.delete_audio(id, || Ok(())).unwrap());

    assert!(!store.reopen(id).unwrap());
    assert_eq!(store.get(id).unwrap().unwrap().status, MeetingStatus::Ready);
}

#[test]
fn transcribing_again_forgets_the_pieces_and_the_summary_goes_with_the_words() {
    let (_dir, store) = store();
    let id = recorded(&store);
    store
        .plan_chunks(id, Track::Mic, &plan(&[(0, 60_000, 30_000)]))
        .unwrap();
    store.chunk_done(id, Track::Mic, 0, "Bonjour.").unwrap();
    let transcript = |text: &str| {
        store
            .save_transcript(
                id,
                &[],
                &[MeetingTurn {
                    speaker_key: "me".into(),
                    start_ms: 0,
                    end_ms: 1_000,
                    text: text.into(),
                    low_confidence: false,
                }],
                SpeakerAnalysis::NoModels,
                None,
                None,
            )
            .unwrap()
    };
    transcript("Bonjour.");
    store.finish_processing(id, None).unwrap();
    store.rename(id, Some("Kept")).unwrap();
    store.save_summary(id, Some("Ignored"), "Notes.").unwrap();

    assert!(store.retranscribe(id).unwrap());
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Processing);
    // The audio may turn out unreadable: the summary waits for the
    // new transcript.
    assert_eq!(meeting.summary.as_deref(), Some("Notes."));
    assert!(store.chunks(id).unwrap().is_empty());
    // Already waiting: a second request changes nothing.
    assert!(!store.retranscribe(id).unwrap());

    // The same words keep their summary; other words get a new one.
    transcript("Bonjour.");
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    transcript("Bonsoir.");
    let meeting = store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary, None);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.title.as_deref(), Some("Kept"));
}
