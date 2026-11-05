use super::*;

#[test]
fn each_sentence_goes_to_the_speaker_who_said_it() {
    let h = harness();
    let id = dialogue_meeting(&h);
    let analyzer = Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true));
    h.process_with(id, codex(|_| Ok(DIALOGUE.join(" "))), Arc::clone(&analyzer));

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
    assert_eq!(
        turns(&meeting),
        [
            ("s1", DIALOGUE[0]),
            ("s2", DIALOGUE[1]),
            ("s1", DIALOGUE[2]),
            ("s2", DIALOGUE[3]),
            ("s1", DIALOGUE[4]),
            ("s2", DIALOGUE[5]),
            // The diarizer heard nobody during "Merci.", and the turn
            // that ended closest is the other voice's.
            (
                "s1",
                "Très bien, je m'occupe de les relancer aujourd'hui. Merci."
            ),
            ("s2", "Dernier point, le séminaire est décalé en mars."),
        ]
    );
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1", "Speaker 2"]);
    assert!(h
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.stage == MeetingStage::Aligning));

    // The saved analysis is reused: rebuilding runs nothing again.
    assert!(h.store.reopen(id).unwrap());
    let again = Arc::new(FakeAnalyzer::without_models());
    let silent = codex(|_| Ok("never".to_string()));
    let verdict = h.process_with(id, Arc::clone(&silent), Arc::clone(&again));
    // Codex was not asked, so this pass says nothing about its limit.
    assert_eq!(verdict, CodexVerdict::Unknown);
    assert_eq!(again.calls.load(Ordering::SeqCst), 0);
    assert!(silent.calls.lock().unwrap().is_empty());
    assert_eq!(turns(&h.store.get(id).unwrap().unwrap()).len(), 8);
}

#[test]
fn a_crashed_analysis_keeps_every_word_under_one_speaker_and_runs_again() {
    let h = harness();
    let id = dialogue_meeting(&h);
    // The helper printed the words, then died before the diarization.
    let words_only = FIXTURE.lines().next().unwrap();
    h.process_with(
        id,
        codex(|_| Ok(DIALOGUE.join(" "))),
        Arc::new(FakeAnalyzer::from_fixture(words_only, false)),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Failed));
    assert!(meeting.turns.iter().all(|turn| turn.speaker_key == "s1"));
    let text: Vec<&str> = meeting
        .turns
        .iter()
        .map(|turn| turn.text.as_str())
        .collect();
    assert_eq!(text.join(" "), DIALOGUE.join(" "));

    // Re-analysing runs the helper again, since it did not finish.
    assert!(h.store.reopen(id).unwrap());
    let full = Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true));
    h.process_with(id, codex(|_| Ok("never".to_string())), Arc::clone(&full));
    assert_eq!(full.calls.load(Ordering::SeqCst), 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
    assert_eq!(meeting.speakers.len(), 2);
}

fn name_of<'a>(meeting: &'a crate::meetings::store::MeetingDetail, text: &str) -> &'a str {
    let turn = meeting
        .turns
        .iter()
        .find(|turn| turn.text.contains(text))
        .unwrap();
    let speaker = meeting
        .speakers
        .iter()
        .find(|speaker| speaker.key == turn.speaker_key)
        .unwrap();
    &speaker.name
}

#[test]
fn a_renamed_voice_keeps_its_name_when_a_retry_fills_an_earlier_part() {
    let h = harness();
    // 25 minutes of remote speech: three pieces.
    let id = h.meeting(MeetingTracks::System, 25 * 60);
    let path = paths::meeting_dir(&h.root, id).join(paths::SYSTEM_TRACK);
    track(&path, 25 * 60, &[(0, 25 * 60)]);
    let levels = track_audio::levels(&path).unwrap();
    let plan = plan_chunks(&levels.levels, levels.duration_ms);
    h.store.plan_chunks(id, Track::System, &plan).unwrap();
    // A first voice speaks during the first piece, a second one after.
    let first_end = plan[0].end_ms as f64 / 1000.0;
    let analysis = Analysis {
        diarized: Some(Track::System),
        turns: Some(vec![
            Turn {
                spk: "A".into(),
                s: 0.0,
                e: first_end,
            },
            Turn {
                spk: "B".into(),
                s: first_end,
                e: 25.0 * 60.0,
            },
        ]),
        complete: true,
        ..Analysis::default()
    };
    // Every piece but the first was transcribed; the first is refused.
    for idx in 1..plan.len() {
        h.store
            .chunk_done(id, Track::System, idx as u32, "Deuxième voix.")
            .unwrap();
    }
    h.process_with(
        id,
        codex(|_| Err(FailureKind::Rejected)),
        Arc::new(FakeAnalyzer::answering(AnalyzerOutcome::Ran(analysis))),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::PartsFailed));
    let second = meeting
        .turns
        .iter()
        .find(|turn| turn.text.contains("Deuxième"))
        .unwrap()
        .speaker_key
        .clone();
    h.store.rename_speaker(id, &second, "Bob").unwrap();

    // Retry transcribes the first piece, where the first voice speaks.
    h.process(id, codex(|_| Ok("Première voix.".to_string())));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(name_of(&meeting, "Deuxième"), "Bob");
    assert_eq!(name_of(&meeting, "Première"), "Speaker 1");
}

#[test]
fn separating_speakers_later_gives_the_new_voices_default_names() {
    let h = harness();
    let id = dialogue_meeting(&h);
    h.process(id, codex(|_| Ok(DIALOGUE.join(" "))));
    // Without the models, the name stands for every remote voice.
    h.store.rename_speaker(id, "s1", "Client team").unwrap();

    assert!(h.store.reopen(id).unwrap());
    h.process_with(
        id,
        codex(|_| Ok("never".to_string())),
        Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true)),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1", "Speaker 2"]);
}

#[test]
fn analysing_again_with_the_same_result_keeps_the_names() {
    let h = harness();
    let id = dialogue_meeting(&h);
    h.process_with(
        id,
        codex(|_| Ok(DIALOGUE.join(" "))),
        Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true)),
    );
    h.store.rename_speaker(id, "s2", "Bob").unwrap();

    // "Separate Speakers" deletes the analysis and runs the models again.
    assert!(h.store.reopen(id).unwrap());
    std::fs::remove_file(paths::meeting_dir(&h.root, id).join(ANALYSIS_FILE)).unwrap();
    h.process_with(
        id,
        codex(|_| Ok("never".to_string())),
        Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true)),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(name_of(&meeting, DIALOGUE[1]), "Bob");
    assert_eq!(name_of(&meeting, DIALOGUE[0]), "Speaker 1");
}

#[test]
fn without_models_each_track_has_one_speaker() {
    let h = harness();
    let id = dialogue_meeting(&h);
    h.process(id, codex(|_| Ok(DIALOGUE.join(" "))));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::NoModels));
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1"]);
}
