use super::decision::{Failure, NotOpened, Skip};
use super::fake_server::{
    closed, segment, serve, session_error, started, transcript_failed, FakeServer, Received, Reply,
    Script,
};
use super::health::{Health, Setback};
use super::live::{pair, LiveDictation, Stop, Timing};
use super::session::{Credentials, End, Params, State};
use super::{plan, Parked, Wanted};
use crate::audio_toolkit::FrameTap;
use crate::codex_transcribe::to_pcm16;
use crate::{OperationId, ProcessingOperation};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

mod listener;

/// What the recorder hands the tap: 30 ms.
const FRAME: usize = 480;
const SECOND: usize = 16_000;

const HOUR: Duration = Duration::from_secs(60 * 60);
/// Waits that only an event or a cancel can end.
const PATIENT: Timing = Timing {
    start: HOUR,
    flush: HOUR,
    close: HOUR,
};

const NO_LANGUAGE: Stop<'static> = Stop {
    audio_raised: false,
    language: None,
};

/// A dictation that streams to `server`.
fn open(server: &FakeServer, language: Option<&str>) -> (FrameTap, LiveDictation) {
    server.dictation(language)
}

/// A ramp, so a sample lost or out of order shows.
fn speech(samples: usize) -> Vec<f32> {
    (0..samples).map(|i| (i % 2_000) as f32 / 4_000.0).collect()
}

/// As the recorder does: a frame at a time.
fn record(tap: &mut FrameTap, samples: &[f32]) {
    for frame in samples.chunks(FRAME) {
        tap(frame);
    }
}

fn operation() -> ProcessingOperation {
    ProcessingOperation::new(OperationId(1))
}

fn outcome(live: LiveDictation, stop: &Stop<'_>, timing: Timing) -> Result<String, Skip> {
    tauri::async_runtime::block_on(live.outcome(stop, &operation(), timing))
}

fn text(live: LiveDictation, stop: &Stop<'_>, health: &Health) -> Option<String> {
    tauri::async_runtime::block_on(live.text_with(stop, &operation(), PATIENT, health))
}

fn audio_messages(heard: &[Received]) -> Vec<&Vec<i16>> {
    heard
        .iter()
        .filter_map(|received| match received {
            Received::Audio(samples) => Some(samples),
            _ => None,
        })
        .collect()
}

/// What the server must have heard of `recording`: its samples as PCM16,
/// then the 250 ms of silence that ends the audio.
fn pcm_then_silence(recording: &[f32]) -> Vec<i16> {
    let mut expected: Vec<i16> = recording.iter().map(|sample| to_pcm16(*sample)).collect();
    expected.extend([0; SECOND / 4]);
    expected
}

fn heard_audio(heard: &[Received]) -> Vec<i16> {
    audio_messages(heard)
        .into_iter()
        .flatten()
        .copied()
        .collect()
}

fn now() -> SystemTime {
    SystemTime::now()
}

#[test]
fn a_stream_gives_the_text_and_the_server_heard_exactly_the_recording() {
    let server = serve(Script::transcribing("Bonjour à tous"));
    let (mut tap, live) = open(&server, Some("fr"));
    let recording = speech(10 * FRAME + 200);
    record(&mut tap, &recording);
    drop(tap);
    let health = Health::new();
    health.note(Setback::Failed, now());

    let stop = Stop {
        audio_raised: false,
        language: Some("fr"),
    };
    assert_eq!(
        text(live, &stop, &health).as_deref(),
        Some("Bonjour à tous.")
    );
    assert_eq!(
        health.setback(now()),
        None,
        "a used stream clears the setback"
    );

    let heard = server.heard();
    let Received::Start(start) = &heard[0] else {
        panic!("the session starts first: {heard:?}");
    };
    assert_eq!(start["config"]["language"], "fr");
    assert_eq!(start["config"]["transcript_delivery_mode"], "final_only");
    assert_eq!(heard_audio(&heard), pcm_then_silence(&recording));
    let sizes: Vec<usize> = audio_messages(&heard).iter().map(|m| m.len()).collect();
    assert_eq!(sizes, [1_920, 1_920, 1_160, 4_000]);
    assert_eq!(heard[heard.len() - 2..], [Received::Close, Received::Gone]);
}

#[test]
fn the_handshake_carries_the_session_and_names_no_client() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let directory = tempfile::tempdir().unwrap();
    let access_token = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD
            .encode(serde_json::json!({"exp": chrono::Utc::now().timestamp() + 3600}).to_string())
    );
    let auth_path = directory.path().join("auth.json");
    let tokens = serde_json::json!({
        "access_token": access_token,
        "refresh_token": "rt",
        "account_id": "account-1",
    });
    std::fs::write(
        &auth_path,
        serde_json::json!({ "tokens": tokens }).to_string(),
    )
    .unwrap();
    let server = serve(Script::transcribing("Bonjour."));
    let (tap, mut live) = pair();
    live.settle(Ok(Params {
        credentials: Credentials::AuthFile(Some(auth_path)),
        ..server.params(None)
    }));
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT).as_deref(),
        Ok("Bonjour.")
    );

    let head = server.handshakes.recv().unwrap();
    let has = |line: &str| head.lines().any(|sent| sent == line);
    assert!(has("sec-websocket-protocol: chatgpt-dictation"), "{head}");
    assert!(
        has(&format!("authorization: bearer {access_token}").to_lowercase()),
        "{head}"
    );
    assert!(has("chatgpt-account-id: account-1"), "{head}");
    assert!(head.contains("user-agent: murmur/"), "{head}");
    for absent in [
        "originator",
        "oai-product-sku",
        "codex",
        "client_type",
        "openai-bearer",
    ] {
        assert!(!head.contains(absent), "{absent} in {head}");
    }
    let heard = server.heard();
    let Received::Start(start) = &heard[0] else {
        panic!("the session starts first: {heard:?}");
    };
    assert!(start["config"].get("language").is_none(), "{start}");
}

#[test]
fn audio_recorded_before_the_session_started_is_sent_once_it_starts() {
    let (release, hold) = mpsc::channel();
    let server = serve(Script {
        hold_start: Some(hold),
        ..Script::transcribing("Bonjour.")
    });
    let (mut tap, live) = open(&server, None);
    let recording = speech(3 * SECOND);
    record(&mut tap, &recording);
    drop(tap);
    // The whole recording waits: the server holds its answer to the start.
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Start(_)
    ));
    release.send(()).unwrap();

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT).as_deref(),
        Ok("Bonjour.")
    );
    assert_eq!(heard_audio(&server.heard()), pcm_then_silence(&recording));
}

#[test]
fn a_refused_handshake_keeps_the_stream_off_for_an_hour_and_is_not_tried_again() {
    let server = serve(Script {
        refuse: Some(403),
        ..Script::default()
    });
    let (tap, live) = open(&server, None);
    drop(tap);
    let health = Health::new();

    assert_eq!(text(live, &NO_LANGUAGE, &health), None);

    assert_eq!(health.setback(now()), Some(Setback::Refused));
    server.handshakes.recv().unwrap();
    assert!(
        server
            .handshakes
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "the handshake was tried a second time"
    );
}

#[test]
fn a_rejected_token_is_a_failure_of_five_minutes() {
    let server = serve(Script {
        refuse: Some(401),
        ..Script::default()
    });
    let (tap, live) = open(&server, None);
    drop(tap);
    let health = Health::new();

    assert_eq!(text(live, &NO_LANGUAGE, &health), None);

    assert_eq!(health.setback(now()), Some(Setback::Failed));
}

#[test]
fn a_server_that_cannot_be_reached_is_a_failure() {
    let server = serve(Script::default());
    let unreachable = Params {
        url: "ws://127.0.0.1:9/backend-api/dictation/stream".to_string(),
        ..server.params(None)
    };
    let (tap, mut live) = pair();
    live.settle(Ok(unreachable));
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT),
        Err(Skip::Failed(Failure::Connect))
    );
}

#[test]
fn each_way_a_session_goes_wrong_leaves_the_dictation_to_the_upload() {
    let well = || Script::transcribing("Bonjour.");
    // (what happens, the script, whether the recording stopped, the failure)
    let rows = [
        (
            "a fatal error while recording",
            Script {
                on_audio: vec![session_error(true)],
                ..well()
            },
            false,
            Failure::SessionError,
        ),
        (
            "an error that answers the start",
            Script {
                on_start: vec![session_error(false)],
                ..well()
            },
            true,
            Failure::SessionError,
        ),
        (
            "a transcript that failed",
            Script {
                on_close: vec![transcript_failed()],
                ..well()
            },
            true,
            Failure::TranscriptFailed,
        ),
        (
            "a frame that is no event",
            Script {
                on_audio: vec![Reply::Event("{}".to_string())],
                ..well()
            },
            false,
            Failure::Unreadable,
        ),
        (
            "a connection dropped while recording",
            Script {
                on_audio: vec![Reply::Hangup],
                ..well()
            },
            false,
            Failure::ClosedEarly,
        ),
        (
            "a session the server closed while recording",
            Script {
                on_audio: vec![closed()],
                ..well()
            },
            false,
            Failure::ClosedEarly,
        ),
        (
            "speech left without its final text",
            Script {
                on_close: vec![segment("utt_1", 1, " Bon"), closed()],
                ..well()
            },
            true,
            Failure::Unfinished,
        ),
    ];
    for (case, script, stopped, failure) in rows {
        let server = serve(script);
        let (mut tap, live) = open(&server, None);
        record(&mut tap, &speech(4 * FRAME));
        let still_recording = (!stopped).then_some(tap);

        assert_eq!(
            outcome(live, &NO_LANGUAGE, PATIENT),
            Err(Skip::Failed(failure)),
            "{case}"
        );
        drop(still_recording);
    }
}

#[test]
fn a_session_that_does_not_start_in_time_is_given_up() {
    let server = serve(Script::default());
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);
    let impatient = Timing {
        start: Duration::from_millis(50),
        ..PATIENT
    };

    assert_eq!(
        outcome(live, &NO_LANGUAGE, impatient),
        Err(Skip::NotStarted)
    );
}

#[test]
fn a_session_that_does_not_close_in_time_is_given_up_and_its_socket_closed() {
    let server = serve(Script {
        on_close: Vec::new(),
        ..Script::transcribing("Bonjour.")
    });
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);
    let impatient = Timing {
        close: Duration::from_millis(50),
        ..PATIENT
    };
    let health = Health::new();

    let skipped = tauri::async_runtime::block_on(live.text_with(
        &NO_LANGUAGE,
        &operation(),
        impatient,
        &health,
    ));

    assert_eq!(skipped, None);
    assert_eq!(health.setback(now()), Some(Setback::Failed));
    let heard = server.heard();
    assert_eq!(heard[heard.len() - 2..], [Received::Close, Received::Gone]);
}

#[test]
fn audio_left_at_the_stop_that_does_not_go_out_in_time_is_given_up() {
    let (release, hold) = mpsc::channel();
    let server = serve(Script {
        hold_start: Some(hold),
        ..Script::transcribing("Bonjour.")
    });
    let (mut tap, live) = open(&server, None);
    // The recording goes on: its end never comes, as audio that is never
    // all sent.
    record(&mut tap, &speech(4 * FRAME));
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Start(_)
    ));
    release.send(()).unwrap();
    let impatient = Timing {
        flush: Duration::from_millis(50),
        ..PATIENT
    };

    assert_eq!(outcome(live, &NO_LANGUAGE, impatient), Err(Skip::Lagging));
    drop(tap);
}

#[test]
fn more_than_three_seconds_unsent_at_the_stop_are_not_waited_for() {
    // The session never starts, so everything recorded is unsent.
    let server = serve(Script::default());
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(3 * SECOND + FRAME));
    drop(tap);

    // No wait could end: the audio alone decides.
    assert_eq!(outcome(live, &NO_LANGUAGE, PATIENT), Err(Skip::Lagging));
}

#[test]
fn a_stream_a_minute_behind_is_given_up_while_recording() {
    let (release, hold) = mpsc::channel();
    let server = serve(Script {
        hold_start: Some(hold),
        ..Script::transcribing("Bonjour.")
    });
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &vec![0.1; 60 * SECOND + FRAME]);
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Start(_)
    ));
    release.send(()).unwrap();

    let mut state = live.state().expect("an open stream");
    tauri::async_runtime::block_on(async {
        while !matches!(*state.borrow_and_update(), State::Done(_)) {
            state.changed().await.unwrap();
        }
    });
    assert_eq!(*state.borrow(), State::Done(End::Lagging));
    assert_eq!(server.heard(), [Received::Gone], "nothing was sent");
    assert_eq!(outcome(live, &NO_LANGUAGE, PATIENT), Err(Skip::Lagging));
    drop(tap);
}

#[test]
fn dropping_a_stream_closes_its_socket_without_closing_the_session() {
    let server = serve(Script::transcribing("Bonjour."));
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(4 * FRAME));
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Start(_)
    ));
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Audio(_)
    ));

    drop(live);
    // As a cancel does: the stream first, then the recorder, whose stop
    // ends the audio.
    drop(tap);

    assert_eq!(server.heard(), [Received::Gone]);
}

#[test]
fn a_cancel_ends_the_wait_for_the_session_at_once() {
    let server = serve(Script {
        on_close: Vec::new(),
        ..Script::transcribing("Bonjour.")
    });
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);
    let operation = operation();
    let cancelling = operation.clone();
    let cancel = thread::spawn(move || {
        // The session is waiting for its close, which never comes.
        while server.received.recv().unwrap() != Received::Close {}
        assert!(cancelling.cancel());
        server
    });

    let skipped = tauri::async_runtime::block_on(live.outcome(&NO_LANGUAGE, &operation, PATIENT));

    assert_eq!(skipped, Err(Skip::Cancelled));
    let server = cancel.join().unwrap();
    assert_eq!(server.heard(), [Received::Gone]);
}

#[test]
fn a_cancelled_dictation_does_not_ask_its_stream() {
    let server = serve(Script::transcribing("Bonjour."));
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);
    let operation = operation();
    operation.cancel();
    let health = Health::new();

    let skipped =
        tauri::async_runtime::block_on(live.text_with(&NO_LANGUAGE, &operation, PATIENT, &health));

    assert_eq!(skipped, None);
    assert_eq!(health.setback(now()), None, "a cancel teaches nothing");
}

#[test]
fn raised_audio_or_another_language_closes_the_stream_unasked() {
    let rows = [
        (
            Stop {
                audio_raised: true,
                language: Some("fr"),
            },
            Skip::QuietSpeech,
        ),
        (
            Stop {
                audio_raised: false,
                language: Some("en"),
            },
            Skip::LanguageChanged,
        ),
        (NO_LANGUAGE, Skip::LanguageChanged),
    ];
    for (stop, skip) in rows {
        let server = serve(Script::transcribing("Bonjour."));
        let (mut tap, live) = open(&server, Some("fr"));
        record(&mut tap, &speech(4 * FRAME));
        assert!(matches!(
            server.received.recv().unwrap(),
            Received::Start(_)
        ));
        assert!(matches!(
            server.received.recv().unwrap(),
            Received::Audio(_)
        ));

        assert_eq!(outcome(live, &stop, PATIENT), Err(skip));
        drop(tap);
        assert_eq!(server.heard(), [Received::Gone], "{skip}");
    }
}

#[test]
fn a_session_with_no_text_leaves_the_clip_to_the_upload_and_teaches_nothing() {
    let server = serve(Script {
        on_start: vec![started()],
        on_close: vec![closed()],
        ..Script::default()
    });
    let (mut tap, live) = open(&server, None);
    record(&mut tap, &vec![0.0; 4 * FRAME]);
    drop(tap);
    let health = Health::new();
    health.note(Setback::Refused, now());

    assert_eq!(text(live, &NO_LANGUAGE, &health), None);

    assert_eq!(health.setback(now()), Some(Setback::Refused));
}

#[test]
fn a_dictation_that_was_not_to_stream_says_why() {
    let (mut tap, mut live) = pair();
    record(&mut tap, &speech(FRAME));
    live.settle(Err(NotOpened::SettingOff));
    // Only the first decision counts.
    live.settle(Err(NotOpened::SignedOut));
    record(&mut tap, &speech(FRAME));
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT),
        Err(Skip::NotOpened(NotOpened::SettingOff))
    );
}

#[test]
fn a_recording_stopped_before_anything_decided_has_no_stream() {
    let (tap, live) = pair();
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT),
        Err(Skip::NotOpened(NotOpened::Undecided))
    );
}

#[test]
fn a_dictation_streams_only_when_everything_allows_it() {
    let wanted = || Wanted {
        enabled: true,
        cloud: true,
        signed_in: true,
        language: "fr-FR",
        listener: None,
    };
    assert_eq!(plan(&wanted(), None), Ok(Some("fr".to_string())));
    let auto = Wanted {
        language: "auto",
        ..wanted()
    };
    assert_eq!(plan(&auto, None), Ok(None));
    let chinese = Wanted {
        language: "zh-Hant",
        ..wanted()
    };
    assert_eq!(plan(&chinese, None), Ok(Some("zh".to_string())));

    let off = Wanted {
        enabled: false,
        ..wanted()
    };
    let local = Wanted {
        cloud: false,
        ..wanted()
    };
    let signed_out = Wanted {
        signed_in: false,
        ..wanted()
    };
    assert_eq!(plan(&off, None), Err(NotOpened::SettingOff));
    assert_eq!(plan(&local, None), Err(NotOpened::OtherEngine));
    assert_eq!(plan(&signed_out, None), Err(NotOpened::SignedOut));
    assert_eq!(
        plan(&wanted(), Some(Setback::Refused)),
        Err(NotOpened::Health(Setback::Refused))
    );

    let nothing = Wanted {
        enabled: false,
        cloud: false,
        signed_in: false,
        language: "auto",
        listener: None,
    };
    assert_eq!(
        plan(&nothing, Some(Setback::Failed)),
        Err(NotOpened::SettingOff),
        "the setting is looked at first"
    );
}

#[test]
fn the_parked_stream_is_the_one_opened_then_taken() {
    let server = serve(Script::transcribing("Bonjour."));
    let parked = Parked::new();
    let (tap, live) = pair();
    parked.park(live);
    parked.settle(Ok(server.params(None)));
    drop(tap);

    let live = parked.take().expect("the parked stream");
    assert!(parked.take().is_none(), "it is taken once");
    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT).as_deref(),
        Ok("Bonjour.")
    );
}

#[test]
fn parking_a_stream_closes_the_one_an_earlier_dictation_left() {
    let server = serve(Script::transcribing("Bonjour."));
    let parked = Parked::new();
    let (mut stale_tap, stale) = open(&server, None);
    record(&mut stale_tap, &speech(4 * FRAME));
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Start(_)
    ));
    assert!(matches!(
        server.received.recv().unwrap(),
        Received::Audio(_)
    ));
    parked.park(stale);

    let (_tap, live) = pair();
    parked.park(live);
    drop(stale_tap);

    assert_eq!(server.heard(), [Received::Gone]);
}

#[test]
fn nothing_is_opened_when_no_stream_is_parked() {
    let server = serve(Script::transcribing("Bonjour."));
    let parked = Parked::new();

    parked.settle(Ok(server.params(None)));

    assert!(server
        .handshakes
        .recv_timeout(Duration::from_millis(300))
        .is_err());
}

/// Opens the real stream with the stored session, never refreshing it, and
/// closes the session without sending any audio, so nothing is transcribed.
#[test]
#[ignore = "talks to chatgpt.com: cargo test --lib live_stream_handshake -- --ignored --nocapture"]
fn live_stream_handshake() {
    use super::protocol::{ClientMessage, ServerEvent};
    use super::session::{event_of, open as open_session, send};
    use futures_util::StreamExt;

    let auth_path = crate::codex_auth::auth_path().unwrap();
    let session = crate::codex_auth::stored_session(&auth_path).unwrap();
    let params = Params {
        url: super::STREAM_URL.to_string(),
        credentials: Credentials::Session(session),
        language: None,
        listener: None,
    };
    tauri::async_runtime::block_on(async {
        let opened = std::time::Instant::now();
        let mut socket = open_session(&params).await.expect("the handshake");
        println!("handshake in {} ms", opened.elapsed().as_millis());
        let mut close_sent = false;
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
                .await
                .expect("an event within 10 s");
            match event_of(frame).expect("an event") {
                ServerEvent::Started => {
                    println!("session.started after {} ms", opened.elapsed().as_millis());
                    send(&mut socket, &ClientMessage::Close).await.unwrap();
                    close_sent = true;
                }
                ServerEvent::Updated { session } if session.is_closed() => break,
                _ => {}
            }
        }
        assert!(close_sent, "the session closed before it started");
        println!("closed after {} ms", opened.elapsed().as_millis());
    });
}

/// Streams the 16 kHz mono PCM16 WAV that `MURMUR_LIVE_WAV` names to the
/// real server, at the pace a microphone would, with the stored session,
/// which it never refreshes. `MURMUR_LIVE_LANGUAGE` names the language, or
/// the server detects it. Prints timings and the word count, never the text.
#[test]
#[ignore = "uses the ChatGPT dictation allowance: MURMUR_LIVE_WAV=<wav> cargo test --lib live_stream_transcribes -- --ignored --nocapture"]
fn live_stream_transcribes() {
    use tokio::time::Instant;

    let clip = std::env::var("MURMUR_LIVE_WAV").expect("MURMUR_LIVE_WAV names a WAV file");
    let mut reader = hound::WavReader::open(clip).unwrap();
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (16_000, 1, 16),
        "a 16 kHz mono PCM16 clip"
    );
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
        .collect();
    let language = std::env::var("MURMUR_LIVE_LANGUAGE").ok();
    let auth_path = crate::codex_auth::auth_path().unwrap();
    let session = crate::codex_auth::stored_session(&auth_path).unwrap();

    // With MURMUR_LIVE_LISTEN set, the session asks for the text as it is
    // heard: each change is counted, never printed.
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u128, usize)>::new()));
    let listening = std::env::var("MURMUR_LIVE_LISTEN").is_ok();

    tauri::async_runtime::block_on(async {
        let (mut tap, mut live) = pair();
        let opened = Instant::now();
        let listener: Option<super::Listener> = listening.then(|| {
            let heard = std::sync::Arc::clone(&heard);
            Box::new(move |text: &str| {
                let at = opened.elapsed().as_millis();
                heard.lock().unwrap().push((at, text.chars().count()));
            }) as super::Listener
        });
        live.settle(Ok(Params {
            url: super::STREAM_URL.to_string(),
            credentials: Credentials::Session(session),
            language: language.clone(),
            listener,
        }));
        let mut state = live.state().expect("an open stream");
        let steps = tauri::async_runtime::spawn(async move {
            let (mut started, mut close_sent) = (None, None);
            loop {
                match &*state.borrow_and_update() {
                    State::Connecting => {}
                    State::Streaming { since } => started = Some(*since),
                    State::Closing { since } => close_sent = Some(*since),
                    State::Done(_) => break,
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
            (started, close_sent)
        });

        let mut pace = tokio::time::interval(Duration::from_millis(30));
        for frame in samples.chunks(FRAME) {
            pace.tick().await;
            tap(frame);
        }
        drop(tap);
        let stop = Stop {
            audio_raised: false,
            language: language.as_deref(),
        };
        let outcome = live.outcome(&stop, &operation(), Timing::MEASURED).await;
        let answered = Instant::now();

        let (started, close_sent) = steps.await.unwrap();
        let since = |from: Option<Instant>, to: Instant| {
            from.map(|from| to.duration_since(from).as_millis().to_string())
                .unwrap_or_else(|| "?".to_string())
        };
        println!(
            "{:.1} s of audio; session.started {} ms after the start",
            samples.len() as f32 / SECOND as f32,
            started
                .map(|at| at.duration_since(opened).as_millis().to_string())
                .unwrap_or_else(|| "?".to_string())
        );
        println!(
            "text {} ms after session.close",
            since(close_sent, answered)
        );
        if listening {
            let heard = heard.lock().unwrap();
            println!(
                "{} texts heard while speaking; the first {} ms after the start; the longest {} characters",
                heard.len(),
                heard.first().map_or(0, |(at, _)| *at),
                heard.iter().map(|(_, length)| *length).max().unwrap_or(0)
            );
        }
        match outcome {
            Ok(text) => println!(
                "{} words, {} characters",
                text.split_whitespace().count(),
                text.chars().count()
            ),
            Err(skip) => panic!("the stream was not used: {skip}"),
        }
    });
}
