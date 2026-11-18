use super::super::fake_server::final_text;
use super::super::session::{self, End, State};
use super::super::{feed, params, Listener};
use super::*;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// A listener, and what it was told, in order.
fn listener() -> (Listener, Arc<Mutex<Vec<String>>>) {
    let told = Arc::new(Mutex::new(Vec::new()));
    let hears = Arc::clone(&told);
    let listener: Listener = Box::new(move |text| hears.lock().unwrap().push(text.to_string()));
    (listener, told)
}

/// A dictation that streams to `server` and tells `listener` its text.
fn open_heard(server: &FakeServer, listener: Listener) -> (FrameTap, LiveDictation) {
    let (tap, mut live) = pair();
    live.settle(Ok(Params {
        listener: Some(listener),
        ..server.params(None)
    }));
    (tap, live)
}

#[test]
fn a_listener_hears_each_new_text_once_in_order_and_the_final_text_is_unchanged() {
    let server = serve(Script {
        on_start: vec![started()],
        on_audio: vec![
            segment("utt_1", 1, " Bon"),
            segment("utt_1", 2, " Bonjour"),
            // The same text again, then an older revision: nothing new.
            segment("utt_1", 3, " Bonjour"),
            segment("utt_1", 1, " Bon"),
            segment("utt_2", 1, " à tous"),
        ],
        on_close: vec![
            final_text("utt_1", 4, "Bonjour."),
            // A final text that is the partial one changes nothing.
            final_text("utt_2", 2, "à tous"),
            closed(),
        ],
        ..Script::default()
    });
    let (listener, told) = listener();
    let (mut tap, live) = open_heard(&server, listener);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT).as_deref(),
        Ok("Bonjour. à tous.")
    );

    assert_eq!(
        *told.lock().unwrap(),
        ["Bon", "Bonjour", "Bonjour à tous", "Bonjour. à tous"]
    );
    let heard = server.heard();
    let Received::Start(start) = &heard[0] else {
        panic!("the session starts first: {heard:?}");
    };
    assert_eq!(start["config"]["transcript_delivery_mode"], "segment");
}

#[test]
fn speech_heard_without_its_final_text_still_goes_to_the_upload() {
    let server = serve(Script {
        on_close: vec![segment("utt_1", 1, " Bon"), closed()],
        ..Script::transcribing("Bonjour.")
    });
    let (listener, told) = listener();
    let (mut tap, live) = open_heard(&server, listener);
    record(&mut tap, &speech(4 * FRAME));
    drop(tap);

    assert_eq!(
        outcome(live, &NO_LANGUAGE, PATIENT),
        Err(Skip::Failed(Failure::Unfinished))
    );
    assert_eq!(*told.lock().unwrap(), ["Bon"]);
}

#[test]
fn an_aborted_stream_tells_its_listener_nothing_more() {
    // An abort sets the flag, and the task runs on until its next wait: it
    // reads the segment, then the close that ends the session.
    for (aborted, expected) in [(false, vec!["Bon"]), (true, Vec::new())] {
        let server = serve(Script {
            on_start: vec![started(), segment("utt_1", 1, " Bon"), closed()],
            ..Script::default()
        });
        let (listener, told) = listener();
        let (_recording, audio) = feed::channel();
        let (state_tx, state) = watch::channel(State::Connecting);
        let params = Params {
            listener: Some(listener),
            ..server.params(None)
        };

        tauri::async_runtime::block_on(session::run(
            params,
            audio,
            Arc::new(AtomicBool::new(aborted)),
            state_tx,
        ));

        assert_eq!(
            *state.borrow(),
            State::Done(End::Failed(Failure::ClosedEarly))
        );
        assert_eq!(*told.lock().unwrap(), expected, "aborted: {aborted}");
    }
}

#[test]
fn the_listener_a_dictation_wants_is_the_one_its_session_tells() {
    let (listener, told) = listener();
    let wanted = Wanted {
        enabled: true,
        cloud: true,
        signed_in: true,
        language: "auto",
        listener: Some(listener),
    };

    let Ok(params) = params(wanted, None) else {
        panic!("a dictation that streams");
    };

    params.listener.expect("the listener")("Bonjour");
    assert_eq!(*told.lock().unwrap(), ["Bonjour"]);
}
