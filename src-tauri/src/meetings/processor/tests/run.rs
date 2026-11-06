use super::*;

#[test]
fn both_tracks_become_one_chronological_transcript() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 30);
    let dir = paths::meeting_dir(&h.root, id);
    // The remote participant speaks first, then the user answers.
    track(&dir.join(paths::SYSTEM_TRACK), 30, &[(1, 6), (20, 24)]);
    track(&dir.join(paths::MIC_TRACK), 30, &[(8, 14)]);
    // Each track is one 30 s piece; only system audio speaks at 2 s.
    let codex = codex_by_audio(|samples| {
        let speaks_early = samples[2 * RATE..3 * RATE].iter().any(|s| s.abs() > 0.1);
        Ok(if speaks_early {
            "On commence la réunion maintenant ? Merci.".to_string()
        } else {
            "Oui, allons-y.".to_string()
        })
    });
    h.process(id, Arc::clone(&codex));

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.error, None);
    let turns: Vec<(&str, &str)> = meeting
        .turns
        .iter()
        .map(|turn| (turn.speaker_key.as_str(), turn.text.as_str()))
        .collect();
    assert_eq!(
        turns,
        // The long question spans both stretches of remote speech
        // (1–6 s and 20–24 s), so the answer at 8 s comes before "Merci."
        [
            ("s1", "On commence la réunion maintenant ?"),
            ("me", "Oui, allons-y."),
            ("s1", "Merci."),
        ]
    );
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Me", "Speaker 2"]);
    let progress = h.events.0.lock().unwrap();
    let last_transcribing = progress
        .iter()
        .rev()
        .find(|p| p.stage == MeetingStage::Transcribing)
        .unwrap();
    assert_eq!(
        (
            last_transcribing.chunks_done,
            last_transcribing.chunks_total
        ),
        (2, 2)
    );
    assert_eq!(codex.calls.lock().unwrap().len(), 2);
}

#[test]
fn a_failed_part_leaves_a_placeholder_and_a_retry_sends_only_that_part() {
    let h = Harness {
        compress_audio: false,
        ..harness()
    };
    // 25 minutes of speech on one track: three pieces.
    let id = h.meeting(MeetingTracks::Mic, 25 * 60);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 25 * 60, &[(0, 25 * 60)]);
    // Exactly one piece is refused.
    let refuse = Arc::new(AtomicBool::new(true));
    let refusing = Arc::clone(&refuse);
    let first = codex(move |seconds| {
        if refusing.swap(false, Ordering::SeqCst) {
            Err(FailureKind::Rejected)
        } else {
            Ok(format!("Partie de {seconds} secondes."))
        }
    });
    h.process(id, Arc::clone(&first));

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.error, Some(ProcessingError::PartsFailed));
    assert_eq!(first.calls.lock().unwrap().len(), 3);
    let placeholders: Vec<_> = meeting
        .turns
        .iter()
        .filter(|turn| turn.speaker_key == crate::meetings::merge::PLACEHOLDER)
        .collect();
    assert_eq!(placeholders.len(), 1);
    // Every piece lasts at least five minutes.
    assert!(placeholders[0].end_ms - placeholders[0].start_ms >= 5 * 60 * 1000);

    // A restart or Retry sends only the piece that failed.
    let second = codex(|seconds| Ok(format!("Partie de {seconds} secondes.")));
    h.process(id, Arc::clone(&second));
    assert_eq!(second.calls.lock().unwrap().len(), 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.turns.len(), 1);
    assert_eq!(meeting.turns[0].speaker_key, "me");
    assert_eq!(meeting.turns[0].text.matches("Partie").count(), 3);
}

#[test]
fn pieces_already_transcribed_are_not_sent_again_after_a_crash() {
    let h = Harness {
        compress_audio: false,
        ..harness()
    };
    let id = h.meeting(MeetingTracks::Mic, 25 * 60);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 25 * 60, &[(0, 25 * 60)]);
    // A first run planned the pieces and saved the first one, then the
    // app was killed with the meeting still processing.
    let levels = track_audio::levels(&dir.join(paths::MIC_TRACK)).unwrap();
    h.store
        .plan_chunks(
            id,
            Track::Mic,
            &plan_chunks(&levels.levels, levels.duration_ms),
        )
        .unwrap();
    h.store.chunk_done(id, Track::Mic, 0, "Déjà fait.").unwrap();

    let codex = codex(|_| Ok("Nouveau.".to_string()));
    h.process(id, Arc::clone(&codex));

    assert_eq!(codex.calls.lock().unwrap().len(), 2);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert!(meeting.turns[0].text.starts_with("Déjà fait."));
    assert_eq!(meeting.status, MeetingStatus::Ready);
}

#[test]
fn a_lost_connection_waits_offline_and_the_next_run_sends_only_what_is_left() {
    let h = Harness {
        compress_audio: false,
        ..harness()
    };
    // 25 minutes of speech on one track: three pieces.
    let id = h.meeting(MeetingTracks::Mic, 25 * 60);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 25 * 60, &[(0, 25 * 60)]);
    // One piece goes through, then the connection drops.
    let answered = Arc::new(AtomicBool::new(false));
    let first = codex(move |seconds| {
        if answered.swap(true, Ordering::SeqCst) {
            Err(FailureKind::Network)
        } else {
            Ok(format!("Partie de {seconds} secondes."))
        }
    });
    let summarizer = FakeSummarizer::answering(vec![]);
    let verdict = h.process_all(
        id,
        Arc::clone(&first),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );

    assert_eq!(verdict, CodexVerdict::Unreachable);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.error, Some(ProcessingError::Offline));
    let done = |h: &Harness| {
        h.store
            .chunks(id)
            .unwrap()
            .iter()
            .filter(|chunk| chunk.status == ChunkStatus::Done)
            .count()
    };
    assert_eq!(done(&h), 1);

    // The connection is back.
    let second = codex(|seconds| Ok(format!("Partie de {seconds} secondes.")));
    let verdict = h.process_all(
        id,
        Arc::clone(&second),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert_eq!(verdict, CodexVerdict::Answered);
    assert_eq!(second.calls.lock().unwrap().len(), 2);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.error, None);
    assert_eq!(meeting.turns[0].text.matches("Partie").count(), 3);
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(done(&h), 3);
}

#[test]
fn offline_a_ready_model_completes_the_transcript_and_the_summary_waits() {
    let local = codex(|seconds| Ok(format!("Sur ce Mac, {seconds} secondes.")));
    let h = Harness {
        local: Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        ..harness()
    };
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![]);
    let verdict = h.process_all(
        id,
        codex(|_| Err(FailureKind::Network)),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );

    assert_eq!(verdict, CodexVerdict::Unreachable);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert!(meeting.local_fallback_ms > 0);
    assert!(meeting
        .turns
        .iter()
        .all(|turn| turn.text.contains("Sur ce Mac")));
    // Not asked while Codex is out of reach, and written once it is back.
    assert_eq!(summarizer.calls(), 0);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert!(h.store.waiting_for_processing().unwrap().contains(&id));
    h.process_all(
        id,
        codex(|_| Ok("never".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(summarizer.calls(), 1);
}

#[test]
fn silent_pieces_are_never_sent() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 40);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 40, &[(2, 10)]);
    track(&dir.join(paths::SYSTEM_TRACK), 40, &[]);
    let codex = codex(|_| Ok("Seul.".to_string()));
    h.process(id, Arc::clone(&codex));
    assert_eq!(codex.calls.lock().unwrap().len(), 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.turns.len(), 1);
    // A quiet call still carries noise: nothing points at the permission.
    assert!(!meeting.system_audio_silent);
}

/// The user speaks and system audio is pure zeros; `played` says whether
/// another app played sound meanwhile. Returns whether the meeting is
/// flagged.
fn zeros_from_the_system(played: bool) -> bool {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 20);
    if played {
        h.store.set_system_played(id).unwrap();
    }
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 20, &[(2, 10)]);
    zeros(&dir.join(paths::SYSTEM_TRACK), 20);

    h.process(id, codex(|_| Ok("Seul.".to_string())));

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    meeting.system_audio_silent
}

#[test]
fn system_audio_of_pure_zeros_while_another_app_played_is_flagged() {
    assert!(zeros_from_the_system(true));
}

/// A meeting in a room, or a call taken on another device: nothing
/// played on the Mac, so zeros are what it should record.
#[test]
fn system_audio_of_pure_zeros_while_nothing_played_is_not_flagged() {
    assert!(!zeros_from_the_system(false));
}

#[test]
fn the_timing_log_gives_each_stage_and_the_speed() {
    let line = timings_line(
        3_600_000,
        &[
            ("transcription", Duration::from_secs(150)),
            ("analysis", Duration::from_millis(61_500)),
            ("summary", Duration::from_millis(800)),
        ],
        Duration::from_secs(180),
    );
    assert_eq!(
        line,
        "audio 60.0 min, transcription 150.0 s, analysis 61.5 s, summary 0.8 s, \
         total 180.0 s (20x real time)"
    );
    assert!(timings_line(0, &[], Duration::ZERO).ends_with("total 0.0 s"));
}
