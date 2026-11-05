use super::*;

/// A meeting in a room with both tracks recorded: nothing played on the
/// Mac and every voice is on the microphone, so that is where the
/// speakers are told apart.
#[test]
fn the_voices_of_a_room_are_told_apart_on_the_microphone() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 34);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 34, &[(0, 34)]);
    zeros(&dir.join(paths::SYSTEM_TRACK), 34);
    let analyzer = Arc::new(FakeAnalyzer::from_fixture_on(Track::Mic, FIXTURE, true));
    h.process_with(id, codex(|_| Ok(DIALOGUE.join(" "))), Arc::clone(&analyzer));

    assert_eq!(*analyzer.diarize.lock().unwrap(), Some(Track::Mic));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
    let keys: Vec<&str> = turns(&meeting).iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, ["s1", "s2", "s1", "s2", "s1", "s2", "s1", "s2"]);
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1", "Speaker 2"]);
}

/// A notification or a ringtone on the Mac during a meeting held in a
/// room: the voices are still told apart on the microphone, and what the
/// Mac played is a voice of its own, not one of the room's.
#[test]
fn a_short_sound_on_the_mac_keeps_the_voices_of_a_room_apart() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 34);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 34, &[(0, 34)]);
    zeros_but(&dir.join(paths::SYSTEM_TRACK), 34, &[(10, 12)]);
    let analyzer = Arc::new(FakeAnalyzer::from_fixture_on(Track::Mic, FIXTURE, true));
    let codex = codex_by_audio(|samples| {
        let from_the_mac = samples[..RATE].iter().all(|sample| *sample == 0.0);
        Ok(if from_the_mac {
            "Ding.".to_string()
        } else {
            DIALOGUE.join(" ")
        })
    });
    h.process_with(id, codex, Arc::clone(&analyzer));

    assert_eq!(*analyzer.diarize.lock().unwrap(), Some(Track::Mic));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    let turns = turns(&meeting);
    let room: std::collections::HashSet<&str> = turns
        .iter()
        .filter(|(_, text)| *text != "Ding.")
        .map(|(key, _)| *key)
        .collect();
    assert_eq!(room, std::collections::HashSet::from(["s1", "s2"]));
    let mac = turns.iter().find(|(_, text)| *text == "Ding.");
    assert_eq!(mac.map(|(key, _)| *key), Some("s3"));
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1", "Speaker 2", "Speaker 3"]);
}

/// The levels of a track of `seconds` that holds `spoken` seconds of
/// speech and `quiet` the rest of the time.
fn speech_for(seconds: u64, spoken: u64, quiet: f32) -> Audio {
    let frames = |seconds: u64| (seconds * 1_000 / chunking::FRAME_MS) as usize;
    let mut levels = vec![-22.0; frames(spoken)];
    levels.resize(frames(seconds), quiet);
    Audio {
        path: PathBuf::new(),
        threshold: speech_threshold(&levels),
        levels: TrackLevels {
            duration_ms: seconds * 1_000,
            levels,
        },
    }
}

const ROOM: f32 = -58.0;

/// The levels of a track of `seconds`: `quiet`, but for the given
/// `(from, to, db)` stretches, in milliseconds.
fn levels_of(seconds: u64, quiet: f32, sounds: &[(u64, u64, f32)]) -> Audio {
    let levels: Vec<f32> = (0..seconds * 1_000 / chunking::FRAME_MS)
        .map(|frame| {
            let ms = frame * chunking::FRAME_MS;
            sounds
                .iter()
                .find(|(from, to, _)| (*from..*to).contains(&ms))
                .map_or(quiet, |(_, _, db)| *db)
        })
        .collect();
    Audio {
        path: PathBuf::new(),
        threshold: speech_threshold(&levels),
        levels: TrackLevels {
            duration_ms: seconds * 1_000,
            levels,
        },
    }
}

/// A call on the built-in speakers, without the on-device models: the
/// user speaks from 1 to 4 s, the remote side from 6 to 9 s, and the
/// microphone hears it 15 dB lower. Codex writes the echo too.
fn echoed_call(route: OutputRoute) -> Vec<(String, String)> {
    let audio = HashMap::from([
        (
            Track::Mic,
            levels_of(20, -65.0, &[(1_000, 4_000, -20.0), (6_000, 9_000, -40.0)]),
        ),
        (
            Track::System,
            levels_of(20, DIGITAL_SILENCE_DBFS, &[(6_000, 9_000, -25.0)]),
        ),
    ]);
    let piece = |track, text: &str| ChunkRow {
        track,
        idx: 0,
        start_ms: 0,
        end_ms: 20_000,
        status: ChunkStatus::Done,
        text: Some(text.to_string()),
    };
    let chunks = [
        piece(Track::System, "On valide le budget jeudi prochain."),
        piece(
            Track::Mic,
            "Moi je préfère lundi, vraiment. On valide le budget jeudi prochain.",
        ),
    ];
    build_transcript(&chunks, &audio, None, Track::System, None, route)
        .turns
        .into_iter()
        .map(|turn| (turn.speaker_key, turn.text))
        .collect()
}

#[test]
fn an_echo_is_removed_without_words_heard_on_device() {
    assert_eq!(
        echoed_call(OutputRoute::Speakers),
        [
            (
                "me".to_string(),
                "Moi je préfère lundi, vraiment.".to_string()
            ),
            (
                "s1".to_string(),
                "On valide le budget jeudi prochain.".to_string()
            ),
        ]
    );
}

/// On headphones the microphone hears no remote voice: whatever the
/// user said stays, even the remote side's words said back.
#[test]
fn nothing_is_taken_off_the_microphone_of_a_meeting_on_headphones() {
    let turns = echoed_call(OutputRoute::Headphones);
    let mine: Vec<&str> = turns
        .iter()
        .filter(|(key, _)| key == "me")
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(
        mine.join(" "),
        "Moi je préfère lundi, vraiment. On valide le budget jeudi prochain."
    );
}

/// Without words heard on device, an hour-long call where the user
/// presents and three remote participants ask questions for a few
/// minutes at the end: the levels pick system audio.
#[test]
fn a_call_where_the_user_presents_tells_the_remote_voices_apart() {
    let audio = HashMap::from([
        (Track::Mic, speech_for(3_600, 2_700, ROOM)),
        (Track::System, speech_for(3_600, 150, DIGITAL_SILENCE_DBFS)),
    ]);
    assert_eq!(track_by_levels(&audio), Track::System);
}

/// Without words heard on device, a call nobody answered rings on the
/// Mac during an hour-long meeting held in a room: the levels pick the
/// microphone.
#[test]
fn a_call_ringing_on_the_mac_keeps_the_voices_of_a_room_apart() {
    let audio = HashMap::from([
        (Track::Mic, speech_for(3_600, 2_000, ROOM)),
        (Track::System, speech_for(3_600, 30, DIGITAL_SILENCE_DBFS)),
    ]);
    assert_eq!(track_by_levels(&audio), Track::Mic);
}

const PRESENTING: &str = "Voici les chiffres du trimestre.";
const QUESTIONS: [&str; 2] = [
    "On peut revoir le budget de cette année.",
    "Et le planning du projet pour le mois prochain.",
];

/// The words of `text` as heard on device, one every half second from
/// `from` seconds.
fn heard(text: &str, from: f64) -> Vec<Word> {
    text.split_whitespace()
        .enumerate()
        .map(|(i, word)| {
            let s = from + i as f64 * 0.5;
            Word {
                w: word.into(),
                s,
                e: s + 0.4,
            }
        })
        .collect()
}

fn said_by(speaker: &str, s: f64, e: f64) -> Turn {
    Turn {
        spk: speaker.into(),
        s,
        e,
    }
}

/// A call where the user presents, and two remote participants then ask
/// a question each: ten seconds of remote speech, far less than the
/// user's. Returns the meeting and what the helper heard.
fn brief_remote_call(h: &Harness) -> (i64, Analysis) {
    let id = h.meeting(MeetingTracks::Both, 130);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 130, &[(0, 100), (112, 130)]);
    track(&dir.join(paths::SYSTEM_TRACK), 130, &[(100, 110)]);
    let analysis = Analysis {
        words: BTreeMap::from([
            (Track::Mic, heard(PRESENTING, 1.0)),
            (
                Track::System,
                [heard(QUESTIONS[0], 100.0), heard(QUESTIONS[1], 105.0)].concat(),
            ),
        ]),
        diarized: Some(Track::System),
        turns: Some(vec![said_by("A", 100.0, 105.0), said_by("B", 105.0, 110.0)]),
        complete: true,
        ..Analysis::default()
    };
    (id, analysis)
}

fn brief_remote_codex() -> Arc<FakeCodex> {
    codex_by_audio(|samples| {
        let presenting = samples[50 * RATE..51 * RATE].iter().any(|s| s.abs() > 0.1);
        Ok(if presenting {
            PRESENTING.to_string()
        } else {
            QUESTIONS.join(" ")
        })
    })
}

/// The words heard on system audio make it the side of the call whose
/// voices are told apart, however little the remote side spoke: the
/// user stays "Me".
#[test]
fn a_call_where_the_remote_side_speaks_briefly_tells_its_voices_apart() {
    let h = harness();
    let (id, analysis) = brief_remote_call(&h);
    let analyzer = Arc::new(FakeAnalyzer::answering(AnalyzerOutcome::Ran(analysis)));
    h.process_with(id, brief_remote_codex(), Arc::clone(&analyzer));

    assert_eq!(*analyzer.diarize.lock().unwrap(), Some(Track::System));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(
        turns(&meeting),
        [
            ("me", PRESENTING),
            ("s1", QUESTIONS[0]),
            ("s2", QUESTIONS[1])
        ]
    );
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Me", "Speaker 2", "Speaker 3"]);
}

/// An analysis saved on the microphone of such a call, as the levels
/// picked it before, is run again on system audio.
#[test]
fn an_analysis_saved_on_the_track_its_words_rule_out_runs_again() {
    let h = harness();
    let (id, analysis) = brief_remote_call(&h);
    let on_the_mic = Analysis {
        diarized: Some(Track::Mic),
        turns: Some(vec![said_by("A", 0.0, 100.0)]),
        ..analysis.clone()
    };
    on_the_mic.save(&paths::meeting_dir(&h.root, id)).unwrap();
    let analyzer = Arc::new(FakeAnalyzer::answering(AnalyzerOutcome::Ran(analysis)));
    h.process_with(id, brief_remote_codex(), Arc::clone(&analyzer));

    assert_eq!(analyzer.calls.load(Ordering::SeqCst), 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Me", "Speaker 2", "Speaker 3"]);
}

/// A call rings unanswered for 25 seconds on the Mac during a short
/// meeting held in a room: louder than the room in the levels, but
/// nothing was said there, so the room's voices are told apart.
#[test]
fn a_ring_on_the_mac_keeps_the_voices_of_a_room_apart() {
    let h = harness();
    let id = h.meeting(MeetingTracks::Both, 300);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::MIC_TRACK), 300, &[(0, 180)]);
    zeros_but(&dir.join(paths::SYSTEM_TRACK), 300, &[(200, 225)]);
    let analysis = Analysis {
        words: BTreeMap::from([
            (
                Track::Mic,
                [heard(QUESTIONS[0], 10.0), heard(QUESTIONS[1], 100.0)].concat(),
            ),
            (Track::System, Vec::new()),
        ]),
        diarized: Some(Track::Mic),
        turns: Some(vec![said_by("A", 0.0, 90.0), said_by("B", 90.0, 180.0)]),
        complete: true,
        ..Analysis::default()
    };
    let analyzer = Arc::new(FakeAnalyzer::answering(AnalyzerOutcome::Ran(analysis)));
    let codex = codex_by_audio(|samples| {
        let room = samples[50 * RATE..51 * RATE].iter().any(|s| s.abs() > 0.1);
        Ok(if room {
            QUESTIONS.join(" ")
        } else {
            "Ding.".to_string()
        })
    });
    h.process_with(id, codex, Arc::clone(&analyzer));

    assert_eq!(*analyzer.diarize.lock().unwrap(), Some(Track::Mic));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(
        turns(&meeting),
        [("s1", QUESTIONS[0]), ("s2", QUESTIONS[1]), ("s3", "Ding.")]
    );
    let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Speaker 1", "Speaker 2", "Speaker 3"]);
}
