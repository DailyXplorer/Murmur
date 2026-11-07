use super::*;

fn recording_stage(deadline: Instant) -> Stage {
    Stage::Recording(ActiveRecording {
        binding_id: "transcribe".to_string(),
        hotkey_string: "CLI".to_string(),
        deadline,
        limit_warned: false,
    })
}

fn pending_start(deadline: Instant) -> PendingStart {
    PendingStart {
        binding_id: "transcribe".to_string(),
        hotkey_string: "Option+Space".to_string(),
        deadline,
    }
}

#[test]
fn remote_cancel_after_toggle_returns_the_coordinator_to_idle() {
    let mut stage = recording_stage(Instant::now() + MAX_RECORDING_DURATION);
    let mut pending = Some(pending_start(Instant::now()));

    finish_cancel(&mut stage, &mut pending);

    assert!(matches!(stage, Stage::Idle));
    assert!(pending.is_none());
}

#[test]
fn processing_cancel_returns_to_idle_while_audio_is_still_stopping() {
    let mut stage = Stage::Processing {
        operation: ProcessingOperation::new(OperationId(1)),
    };

    finish_cancel(&mut stage, &mut None);

    assert!(matches!(stage, Stage::Idle));
}

#[test]
fn late_cancel_waits_for_paste_release_before_admitting_another_recording() {
    let operation = ProcessingOperation::new(OperationId(1));
    assert!(operation.try_enter_paste().is_some());
    let stage = Stage::Processing { operation };

    assert!(!request_cancel(&stage));
    assert!(matches!(stage, Stage::Processing { .. }));
}

#[test]
fn restore_is_ignored_unless_the_coordinator_is_idle() {
    let operation = ProcessingOperation::new(OperationId(1));
    assert!(operation.try_enter_paste().is_some());

    assert!(admits_restore(&Stage::Idle));
    assert!(!admits_restore(&recording_stage(Instant::now())));
    assert!(!admits_restore(&Stage::Processing {
        operation: ProcessingOperation::new(OperationId(2)),
    }));
    assert!(!admits_restore(&Stage::Processing { operation }));
}

/// `begin_shutdown` needs a live app, so this pins its calls instead: the
/// tray's Quit closes the cancelled-dictation slot before its cancel can
/// make a pipeline keep its audio.
#[test]
fn the_tray_quit_closes_the_cancelled_dictation_before_it_cancels() {
    let begin_shutdown = include_str!("../transcription_coordinator.rs")
        .split("pub fn begin_shutdown")
        .nth(1)
        .expect("the coordinator begins the shutdown");
    let close = begin_shutdown
        .find("recently_cancelled::close(&app)")
        .expect("the quit closes the cancelled dictation");
    let cancel = begin_shutdown
        .find("Command::Shutdown")
        .expect("the quit cancels the running dictation");
    assert!(close < cancel);
}

#[test]
fn quit_waits_for_a_fake_modifier_release_receipt() {
    let operation = ProcessingOperation::new(OperationId(1));
    assert!(operation.try_enter_paste().is_some());
    let stage = Stage::Processing { operation };
    assert!(!request_cancel(&stage));

    let drain = Arc::new(ShutdownDrain::new());
    assert!(drain.begin());
    let waiting_drain = Arc::clone(&drain);
    let (tx, rx) = mpsc::channel();
    let waiter = thread::spawn(move || {
        tx.send(waiting_drain.wait_bounded(Duration::from_secs(5)))
            .unwrap();
    });

    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(10)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drain.finish();
    assert!(rx.recv().unwrap());
    waiter.join().unwrap();
}

#[test]
fn stale_completion_cannot_finish_a_later_operation() {
    let first = OperationId(1);
    let second = OperationId(2);
    let stage = Stage::Processing {
        operation: ProcessingOperation::new(second),
    };

    assert!(!is_current_processing(&stage, first));
    assert!(is_current_processing(&stage, second));
}

#[test]
fn receiver_wakes_for_the_earliest_gesture_start_or_recording_deadline() {
    let now = Instant::now();
    let stage = recording_stage(now + Duration::from_secs(600));
    let waiting = Gesture::AwaitingSecondTap {
        deadline: now + Duration::from_millis(350),
    };
    assert_eq!(
        next_deadline(&stage, waiting, None),
        Some(now + Duration::from_millis(350))
    );
    let retry = pending_start(now + Duration::from_millis(25));
    assert_eq!(
        next_deadline(&stage, waiting, Some(&retry)),
        Some(now + Duration::from_millis(25))
    );
    assert_eq!(next_deadline(&Stage::Idle, Gesture::Idle, None), None);
}

#[test]
fn recording_warns_a_minute_before_the_cap_then_stops_at_it() {
    let now = Instant::now();
    let deadline = now + Duration::from_secs(600);
    let mut stage = recording_stage(deadline);
    let warn_at = deadline - LIMIT_WARNING_LEAD;

    assert_eq!(
        next_deadline(&stage, Gesture::HandsFree { key_down: false }, None),
        Some(warn_at)
    );
    assert!(!take_due_limit_warning(
        &mut stage,
        warn_at - Duration::from_millis(1)
    ));
    assert!(take_due_limit_warning(&mut stage, warn_at));
    assert!(
        !take_due_limit_warning(&mut stage, warn_at),
        "the warning is shown once"
    );
    assert_eq!(
        next_deadline(&stage, Gesture::HandsFree { key_down: false }, None),
        Some(deadline)
    );
    assert_eq!(
        take_due_limit_stop(&stage, deadline - Duration::from_millis(1)),
        None
    );
    assert_eq!(
        take_due_limit_stop(&stage, deadline),
        Some(("transcribe".to_string(), "CLI".to_string()))
    );
}

#[test]
fn deferred_start_waits_for_idle_and_never_creates_a_phantom_recording() {
    let now = Instant::now();
    let mut pending = Some(pending_start(now));

    // Cancellation can leave the coordinator Idle while audio is Stopping.
    // Retrying remains a request until the recorder admits it.
    assert_eq!(
        take_due_start(&Stage::Idle, &mut pending, now),
        Some(("transcribe".to_string(), "Option+Space".to_string()))
    );
    assert!(pending.is_none());

    let mut stale = Some(pending_start(now));
    assert_eq!(take_due_start(&recording_stage(now), &mut stale, now), None);
    assert!(stale.is_none());
}

#[test]
fn a_gesture_outlives_neither_its_recording_nor_its_queued_start() {
    let now = Instant::now();
    assert!(gesture_has_recording(&recording_stage(now), None));
    assert!(gesture_has_recording(
        &Stage::Idle,
        Some(&pending_start(now))
    ));
    assert!(!gesture_has_recording(&Stage::Idle, None));
    assert!(!gesture_has_recording(
        &Stage::Processing {
            operation: ProcessingOperation::new(OperationId(1)),
        },
        None
    ));
}

/// Feeds the queue through `next_step` the way `Worker::run` does, at a
/// fixed `now`, and returns the intents and the final gesture.
fn drain_at(rx: &Receiver<Command>, mut gesture: Gesture, now: Instant) -> (Vec<Intent>, Gesture) {
    let mut intents = Vec::new();
    loop {
        match next_step(rx, gesture, now) {
            Next::Command(Command::Key { key, at, .. }) => {
                let (next, intent) = gesture.on_key(key, at);
                gesture = next;
                intents.push(intent);
            }
            Next::GestureExpired(next) => {
                gesture = next;
                intents.push(Intent::Discard);
            }
            Next::Command(_) => unreachable!("only key commands are queued"),
            Next::Wait | Next::Closed => break,
        }
    }
    intents.retain(|intent| *intent != Intent::None);
    (intents, gesture)
}

fn key_at(key: Key, at: Instant) -> Command {
    Command::Key {
        binding_id: "transcribe".to_string(),
        hotkey_string: "Option+Space".to_string(),
        key,
        at,
    }
}

#[test]
fn a_double_tap_queued_behind_a_slow_microphone_locks_hands_free() {
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel();
    // The first press started a recording, and opening the microphone
    // held the coordinator until 600 ms while the rest of the double tap
    // queued up.
    for (key, at_ms) in [(Key::Release, 80), (Key::Press, 200), (Key::Release, 260)] {
        tx.send(key_at(key, t0 + Duration::from_millis(at_ms)))
            .unwrap();
    }

    let (intents, gesture) = drain_at(
        &rx,
        Gesture::Pressed { since: t0 },
        t0 + Duration::from_millis(600),
    );

    assert_eq!(intents, [Intent::LockHandsFree]);
    assert_eq!(gesture, Gesture::HandsFree { key_down: false });
}

#[test]
fn a_lone_tap_expires_only_once_the_queue_is_empty() {
    let t0 = Instant::now();
    let (tx, rx) = mpsc::channel();
    tx.send(key_at(Key::Release, t0 + Duration::from_millis(80)))
        .unwrap();

    let (intents, gesture) = drain_at(
        &rx,
        Gesture::Pressed { since: t0 },
        t0 + Duration::from_millis(600),
    );

    assert_eq!(intents, [Intent::Discard]);
    assert_eq!(gesture, Gesture::Idle);
}

#[test]
fn presses_closer_than_the_debounce_are_bounces() {
    let t0 = Instant::now();
    assert!(!is_bounce(None, t0));
    assert!(is_bounce(Some(t0), t0 + Duration::from_millis(29)));
    assert!(!is_bounce(Some(t0), t0 + DEBOUNCE));
}

#[test]
fn local_cancel_queues_once_without_waiting_for_the_coordinator() {
    let (tx, rx) = mpsc::channel();

    // `CancelAction` can call send_cancel from the coordinator thread
    // itself. An mpsc send adds one later command and never waits for that
    // thread to receive it, so there is neither recursive dispatch nor a
    // self-deadlock.
    assert!(queue_local_cancel(&tx).is_ok());
    assert!(matches!(rx.try_recv(), Ok(Command::Cancel)));
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}
