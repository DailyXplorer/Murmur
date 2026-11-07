use super::*;
use std::cell::RefCell;
use std::sync::Arc;

#[test]
fn microphone_fallback_changes_only_the_unavailable_selection() {
    let mut settings = crate::settings::get_default_settings();
    settings.selected_microphone = Some("Unavailable USB microphone".to_string());
    settings.selected_channel = Some(2);
    settings.window_names_enabled = false;
    settings.custom_words.push("Murmur".to_string());
    let mut expected = serde_json::to_value(&settings).unwrap();
    expected["selected_microphone"] = serde_json::Value::Null;

    assert!(clear_unavailable_microphone(
        &mut settings,
        "Unavailable USB microphone"
    ));
    assert_eq!(serde_json::to_value(&settings).unwrap(), expected);
}

#[test]
fn a_late_microphone_fallback_preserves_a_newer_selection() {
    let mut settings = crate::settings::get_default_settings();
    settings.selected_microphone = Some("New microphone".to_string());
    let expected = serde_json::to_value(&settings).unwrap();

    assert!(!clear_unavailable_microphone(
        &mut settings,
        "Unavailable USB microphone"
    ));
    assert_eq!(serde_json::to_value(&settings).unwrap(), expected);
}

#[test]
fn late_stop_cannot_claim_a_new_recording_with_the_same_binding() {
    let newer_recording = RecordingState::Recording {
        binding_id: "transcribe".to_string(),
    };

    // Cancellation A advances the generation before B begins. A's worker
    // may run after B has reused the binding, but it cannot invalidate B's
    // readiness, move B to Stopping, or call Recorder::stop.
    assert!(!can_claim_stop(&newer_recording, "transcribe", 8, 9));
    assert!(can_claim_stop(&newer_recording, "transcribe", 9, 9));
}

#[test]
fn a_readiness_thread_cannot_mute_after_cancel_released_the_mute() {
    let system = RefCell::new(Vec::new());
    let mut mute = MuteState::default();

    // Cancel invalidated readiness and released before the readiness
    // thread reached its mute step.
    mute.release(|prev| system.borrow_mut().push(format!("restore {prev:?}")));
    mute.engage(
        false,
        || Some(false),
        || system.borrow_mut().push("mute".into()),
    );

    assert!(system.borrow().is_empty());
    assert!(!mute.did_mute);
}

#[test]
fn no_recording_can_mute_once_quitting_released_the_mute() {
    let system = RefCell::new(Vec::new());
    let mut mute = MuteState::default();

    mute.engage(
        true,
        || Some(false),
        || system.borrow_mut().push("mute".to_string()),
    );
    mute.release_for_good(|prev| system.borrow_mut().push(format!("restore {prev:?}")));
    // A dictation started while quitting has a current generation.
    mute.engage(
        true,
        || Some(false),
        || system.borrow_mut().push("mute".to_string()),
    );

    assert_eq!(*system.borrow(), ["mute", "restore Some(false)"]);
}

#[test]
fn release_restores_the_state_captured_by_the_first_engage() {
    let system = RefCell::new(Vec::new());
    let mut mute = MuteState::default();

    mute.engage(
        true,
        || Some(false),
        || system.borrow_mut().push("mute".to_string()),
    );
    mute.engage(
        true,
        || Some(true),
        || system.borrow_mut().push("mute".to_string()),
    );
    mute.release(|prev| system.borrow_mut().push(format!("restore {prev:?}")));
    mute.release(|prev| system.borrow_mut().push(format!("restore {prev:?}")));

    assert_eq!(*system.borrow(), ["mute", "restore Some(false)"]);
}

#[test]
fn input_changes_require_an_idle_recording_state() {
    assert!(ensure_idle_for_input_change(&RecordingState::Idle, "microphone").is_ok());

    let recording = ensure_idle_for_input_change(
        &RecordingState::Recording {
            binding_id: "main".to_string(),
        },
        "microphone",
    )
    .unwrap_err();
    assert_eq!(
        recording.to_string(),
        "Cannot change the microphone while recording"
    );

    let stopping =
        ensure_idle_for_input_change(&RecordingState::Stopping, "input channel").unwrap_err();
    assert_eq!(
        stopping.to_string(),
        "Cannot change the input channel while recording"
    );
}

#[test]
fn on_demand_to_always_on_waits_for_device_or_channel_transaction_commit() {
    for input_name in ["microphone", "input channel"] {
        let state = Arc::new(Mutex::new(RecordingState::Idle));
        let configured_input = Arc::new(Mutex::new("previous input"));
        let mode = Arc::new(Mutex::new(MicrophoneMode::OnDemand));
        let persisted_modes = Arc::new(Mutex::new(Vec::new()));
        let (transaction_started_tx, transaction_started_rx) = mpsc::sync_channel(0);
        let (commit_transaction_tx, commit_transaction_rx) = mpsc::sync_channel(0);

        let state_for_transaction = Arc::clone(&state);
        let configured_input_for_transaction = Arc::clone(&configured_input);
        let transaction = std::thread::spawn(move || {
            let state = lock_input_reconfiguration(&state_for_transaction);
            ensure_idle_for_input_change(&state, input_name).unwrap();
            transaction_started_tx.send(()).unwrap();

            commit_transaction_rx.recv().unwrap();
            *configured_input_for_transaction.lock().unwrap() = "updated input";
        });

        transaction_started_rx.recv().unwrap();

        let state_for_mode = Arc::clone(&state);
        let configured_input_for_mode = Arc::clone(&configured_input);
        let mode_for_transition = Arc::clone(&mode);
        let persisted_modes_for_transition = Arc::clone(&persisted_modes);
        let (mode_before_lock_tx, mode_before_lock_rx) = mpsc::sync_channel(0);
        let (stream_started_with_tx, stream_started_with_rx) = mpsc::channel();
        let mode_transition = std::thread::spawn(move || {
            let before_reconfiguration_lock = || mode_before_lock_tx.send(()).unwrap();
            update_mode_transaction(
                &state_for_mode,
                &mode_for_transition,
                MicrophoneMode::AlwaysOn,
                || panic!("OnDemand -> AlwaysOn must not stop the stream"),
                || {
                    stream_started_with_tx
                        .send(*configured_input_for_mode.lock().unwrap())
                        .unwrap();
                    Ok(())
                },
                |mode| {
                    persisted_modes_for_transition
                        .lock()
                        .unwrap()
                        .push(matches!(mode, MicrophoneMode::AlwaysOn))
                },
                Some(&before_reconfiguration_lock),
            )
            .unwrap();
        });

        // The exact update-mode transaction has reached the same lock that
        // the input transaction holds. It therefore cannot snapshot or
        // open until this controlled commit finishes.
        mode_before_lock_rx.recv().unwrap();
        commit_transaction_tx.send(()).unwrap();
        transaction.join().unwrap();
        mode_transition.join().unwrap();
        assert_eq!(stream_started_with_rx.recv().unwrap(), "updated input");
        assert!(matches!(*mode.lock().unwrap(), MicrophoneMode::AlwaysOn));
        assert_eq!(*persisted_modes.lock().unwrap(), [true]);
    }
}

#[test]
fn always_on_open_failure_keeps_mode_and_persistence_on_demand() {
    let state = Mutex::new(RecordingState::Idle);
    let mode = Mutex::new(MicrophoneMode::OnDemand);
    let persisted_modes = Mutex::new(Vec::new());

    let result = update_mode_transaction(
        &state,
        &mode,
        MicrophoneMode::AlwaysOn,
        || panic!("OnDemand -> AlwaysOn must not stop the stream"),
        || Err(anyhow::anyhow!("input unavailable")),
        |mode| {
            persisted_modes
                .lock()
                .unwrap()
                .push(matches!(mode, MicrophoneMode::AlwaysOn))
        },
        None,
    );

    assert_eq!(result.unwrap_err().to_string(), "input unavailable");
    assert!(matches!(*mode.lock().unwrap(), MicrophoneMode::OnDemand));
    assert!(persisted_modes.lock().unwrap().is_empty());
}

#[test]
fn always_on_during_a_recording_keeps_the_stream_it_records_on() {
    for recording in [
        RecordingState::Recording {
            binding_id: "main".to_string(),
        },
        RecordingState::Stopping,
    ] {
        let state = Mutex::new(recording);
        let mode = Mutex::new(MicrophoneMode::OnDemand);
        let persisted_modes = Mutex::new(Vec::new());

        update_mode_transaction(
            &state,
            &mode,
            MicrophoneMode::AlwaysOn,
            || panic!("OnDemand -> AlwaysOn must not stop the stream"),
            || panic!("a recording's stream must not be reopened under it"),
            |mode| {
                persisted_modes
                    .lock()
                    .unwrap()
                    .push(matches!(mode, MicrophoneMode::AlwaysOn))
            },
            None,
        )
        .unwrap();

        assert!(matches!(*mode.lock().unwrap(), MicrophoneMode::AlwaysOn));
        assert_eq!(*persisted_modes.lock().unwrap(), [true]);
    }
}

#[test]
fn stream_restart_uses_the_updated_input_after_closing_the_old_stream() {
    let steps = RefCell::new(Vec::new());
    let outcome = restart_open_stream_transaction(
        || steps.borrow_mut().push("stop"),
        || {
            steps.borrow_mut().push("start-updated");
            Ok::<_, anyhow::Error>("updated")
        },
        || unreachable!("the previous stream must not be reopened after a successful update"),
    )
    .unwrap();

    assert_eq!(*steps.borrow(), ["stop", "start-updated"]);
    assert!(matches!(outcome, StreamRestartOutcome::Updated("updated")));
}

#[test]
fn stream_restart_restores_the_previous_input_when_the_update_fails() {
    let steps = RefCell::new(Vec::new());
    let outcome = restart_open_stream_transaction(
        || steps.borrow_mut().push("stop"),
        || {
            steps.borrow_mut().push("start-updated");
            Err(anyhow::anyhow!("new input rejected"))
        },
        || {
            steps.borrow_mut().push("restore-previous");
            Ok::<_, anyhow::Error>("previous")
        },
    )
    .unwrap();

    assert_eq!(
        *steps.borrow(),
        ["stop", "start-updated", "restore-previous"]
    );
    match outcome {
        StreamRestartOutcome::RestoredAfterUpdateFailure {
            restored,
            update_error,
        } => {
            assert_eq!(restored, "previous");
            assert_eq!(update_error.to_string(), "new input rejected");
        }
        StreamRestartOutcome::Updated(_) => {
            panic!("expected the previous stream to be restored")
        }
    }
}

#[test]
fn stream_restart_reports_when_neither_input_can_be_opened() {
    let result = restart_open_stream_transaction::<()>(
        || {},
        || Err(anyhow::anyhow!("new input rejected")),
        || Err(anyhow::anyhow!("previous input rejected")),
    );

    match result {
        Ok(_) => panic!("expected both stream opens to fail"),
        Err(error) => assert_eq!(
            error.to_string(),
            "Failed to apply input change (new input rejected); failed to restore the previous input (previous input rejected)"
        ),
    }
}
