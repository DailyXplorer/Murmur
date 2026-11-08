use super::*;

#[test]
fn monitor_hit_test_uses_half_open_physical_bounds() {
    let position = PhysicalPosition::new(-2560, -200);
    let size = PhysicalSize::new(2560, 1440);

    assert!(is_point_within((-2560, -200), &position, &size));
    assert!(is_point_within((-1, 1239), &position, &size));
    assert!(!is_point_within((0, 0), &position, &size));
    assert!(!is_point_within((-1, 1240), &position, &size));
}

#[test]
fn with_the_overlay_off_a_prompt_over_a_meeting_leaves_nothing_on_screen() {
    let pill = MeetingPill {
        started_at_ms: 1,
        stops_at_ms: 2,
    };
    assert_eq!(resting_pill(Some(pill), OverlayStyle::None), None);
    assert_eq!(resting_pill(Some(pill), OverlayStyle::Minimal), Some(pill));
    assert_eq!(resting_pill(None, OverlayStyle::Minimal), None);
}

#[test]
fn copy_window_wraps_the_measured_capsule_in_its_margins() {
    let screen = (1440.0, 875.0);
    assert_eq!(copy_window_size((300.4, 62.0), screen), (333.0, 94.0));
    assert_eq!(copy_window_size((340.0, 34.0), screen), (372.0, 66.0));
}

#[test]
fn copy_window_stays_between_the_meeting_window_and_the_screen() {
    let screen = (1440.0, 875.0);
    let least = (MEETING_OVERLAY_WIDTH, MEETING_OVERLAY_HEIGHT);
    assert_eq!(copy_window_size((40.0, 12.0), screen), least);
    assert_eq!(copy_window_size((-5.0, 0.0), screen), least);
    assert_eq!(copy_window_size((f64::NAN, f64::INFINITY), screen), least);
    assert_eq!(
        copy_window_size((4000.0, 2000.0), screen),
        (1440.0, 875.0 - OVERLAY_TOP_OFFSET)
    );
}

/// A copy pill of the test's own, not the shared notice, whose Copy
/// writes to the returned pasteboard.
fn copy_pill(text: &str) -> (Option<Notice>, Arc<Mutex<Vec<String>>>) {
    let pasteboard = Arc::new(Mutex::new(Vec::new()));
    let written = Arc::clone(&pasteboard);
    let notice = Some(Notice::Copy {
        text: text.to_string(),
        write: Arc::new(move |text| {
            written.lock().unwrap().push(text.to_string());
            Ok(())
        }),
        mic_lost_next: false,
    });
    (notice, pasteboard)
}

/// Clicks Copy as `copy_held_notice` does.
fn click_copy(notice: &Option<Notice>) -> Result<(), String> {
    match notice {
        Some(Notice::Copy { text, write, .. }) => write(text),
        _ => Err("The copy notice is no longer showing".to_string()),
    }
}

#[test]
fn a_late_transcribing_state_or_warning_leaves_the_copy_pills_text() {
    let (mut notice, pasteboard) = copy_pill("the dictated text");

    assert!(!dictation_takes_overlay(&mut notice, "transcribing"));
    assert!(!hold_mic_lost_notice(&mut notice, &operation(1)));

    assert_eq!(click_copy(&notice), Ok(()));
    assert_eq!(pasteboard.lock().unwrap().as_slice(), ["the dictated text"]);
    assert!(
        dictation_takes_overlay(&mut notice, "recording"),
        "a new recording replaces it"
    );
    assert!(notice.is_none());
}

#[test]
fn a_copy_pill_and_the_microphone_warning_show_one_after_the_other() {
    let (mut both, _) = copy_pill("the dictated text");
    if let Some(Notice::Copy { mic_lost_next, .. }) = both.as_mut() {
        *mic_lost_next = true;
    }
    assert_eq!(
        dismiss(&mut both, Dismissal::Timer(1), 1, OverlayStyle::Minimal),
        Some(AfterDismissal::Warning),
        "the warning shows once the copy pill goes"
    );
    assert_eq!(
        dismiss(&mut both, Dismissal::Timer(2), 2, OverlayStyle::Minimal),
        Some(AfterDismissal::Settled),
        "then nothing"
    );

    for mut alone in [
        copy_pill("text").0,
        Some(Notice::Error),
        Some(Notice::Cancelled),
    ] {
        assert_eq!(
            dismiss(&mut alone, Dismissal::Timer(1), 1, OverlayStyle::Minimal),
            Some(AfterDismissal::Settled)
        );
        assert!(alone.is_none());
    }
}

#[test]
fn a_late_timer_leaves_the_warning_that_followed_the_copy_pill() {
    let (mut notice, _) = copy_pill("the dictated text");
    if let Some(Notice::Copy { mic_lost_next, .. }) = notice.as_mut() {
        *mic_lost_next = true;
    }
    // Escape on the copy pill shows the warning, at generation 2.
    assert_eq!(
        dismiss(&mut notice, Dismissal::Escape, 1, OverlayStyle::Minimal),
        Some(AfterDismissal::Warning)
    );
    assert!(matches!(notice, Some(Notice::MicLost)));

    // The copy pill's timer, started at generation 1, wakes late.
    assert_eq!(
        dismiss(&mut notice, Dismissal::Timer(1), 2, OverlayStyle::Minimal),
        None
    );
    assert!(
        matches!(notice, Some(Notice::MicLost)),
        "the warning keeps its time"
    );
    assert_eq!(
        dismiss(&mut notice, Dismissal::Timer(2), 2, OverlayStyle::Minimal),
        Some(AfterDismissal::Settled)
    );
    assert!(notice.is_none());

    let mut error = Some(Notice::Error);
    assert_eq!(
        dismiss(&mut error, Dismissal::Escape, 3, OverlayStyle::Minimal),
        None,
        "Escape leaves an error"
    );
    assert!(matches!(error, Some(Notice::Error)));
}

#[cfg(target_os = "macos")]
#[test]
fn escape_monitors_only_dismiss_fitted_notices_for_the_escape_key() {
    for key in [0, 36, 51, ESCAPE_KEY_CODE] {
        let (mut notice, _) = copy_pill("text");
        let after = escape_dismissal(key)
            .and_then(|dismissal| dismiss(&mut notice, dismissal, 1, OverlayStyle::Minimal));
        if key == ESCAPE_KEY_CODE {
            assert_eq!(after, Some(AfterDismissal::Settled));
            assert!(notice.is_none());
        } else {
            assert_eq!(after, None);
            assert!(copy_pill_showing(&notice));
        }
    }
    let mut notice = Some(Notice::Error);
    assert_eq!(
        escape_dismissal(ESCAPE_KEY_CODE)
            .and_then(|dismissal| { dismiss(&mut notice, dismissal, 1, OverlayStyle::Minimal) }),
        None
    );
    assert!(matches!(notice, Some(Notice::Error)));
}

/// A copy pill whose warning follows it.
fn copy_pill_then_warning() -> Option<Notice> {
    let (mut notice, _) = copy_pill("the dictated text");
    if let Some(Notice::Copy { mic_lost_next, .. }) = notice.as_mut() {
        *mic_lost_next = true;
    }
    notice
}

#[test]
fn a_copy_click_after_the_timer_showed_the_warning_leaves_the_warning_its_time() {
    assert!(copy_pill_showing(&copy_pill("text").0));
    let mut notice = copy_pill_then_warning();
    dismiss(&mut notice, Dismissal::Timer(1), 1, OverlayStyle::Minimal);
    assert!(
        !copy_pill_showing(&notice),
        "the click arms no Copied timer over the warning"
    );
}

#[test]
fn with_the_overlay_turned_off_nothing_follows_the_copy_pill() {
    let mut notice = copy_pill_then_warning();
    assert_eq!(
        dismiss(&mut notice, Dismissal::Timer(1), 1, OverlayStyle::None),
        Some(AfterDismissal::Settled)
    );
    assert!(notice.is_none());
}

#[test]
fn the_microphone_warning_is_fitted_and_dismissed_like_the_copy_pill() {
    let mut notice = Some(Notice::Error);
    assert!(
        hold_mic_lost_notice(&mut notice, &operation(1)),
        "it replaces an error"
    );
    assert!(matches!(notice, Some(Notice::MicLost)));

    assert!(notice.as_ref().is_some_and(Notice::is_fitted));
    assert!(copy_pill("text").0.as_ref().is_some_and(Notice::is_fitted));
    assert!(!Notice::Error.is_fitted(), "an error pill has a fixed size");

    assert!(!dictation_takes_overlay(&mut notice, "transcribing"));
    assert!(matches!(notice, Some(Notice::MicLost)));
    assert!(dictation_takes_overlay(&mut notice, "recording"));
    assert!(notice.is_none());

    let mut error = Some(Notice::Error);
    assert!(
        dictation_takes_overlay(&mut error, "transcribing"),
        "a dictation still replaces an error"
    );
    assert!(error.is_none());
}

#[test]
fn restore_replaces_the_cancelled_pill_it_was_clicked_from() {
    let mut notice = Some(Notice::Cancelled);
    assert!(notice.as_ref().is_some_and(Notice::is_fitted));
    assert!(dictation_takes_overlay(&mut notice, "transcribing"));
    assert!(notice.is_none());
}

#[test]
fn a_cancelled_dictation_keeps_its_pill_but_never_covers_a_newer_dictation() {
    let shown = DictationEpoch(4);

    // Cancelled while it transcribes: it still owns the overlay when its
    // pipeline unwinds and raises the pill, before the coordinator hides it.
    let mut notice = None;
    assert!(hold_cancelled_notice(&mut notice, shown, DictationEpoch(4)));
    assert!(matches!(notice, Some(Notice::Cancelled)));

    // A newer dictation showed first: its recorder may not be open yet, or
    // it already transcribes. The old cancel's pill is dropped.
    let mut notice = None;
    assert!(dictation_takes_overlay(&mut notice, "recording"));
    assert!(!hold_cancelled_notice(
        &mut notice,
        shown,
        DictationEpoch(5)
    ));
    assert!(notice.is_none());

    // The cancel's pill came first: the new dictation replaces it, and the
    // pill's 5 s timer finds a newer overlay generation.
    let mut notice = None;
    assert!(hold_cancelled_notice(&mut notice, shown, DictationEpoch(4)));
    assert!(dictation_takes_overlay(&mut notice, "recording"));
    assert!(notice.is_none());
}

fn operation(id: u64) -> ProcessingOperation {
    ProcessingOperation::new(crate::OperationId(id))
}

#[test]
fn a_dictation_cancelled_during_its_history_save_leaves_the_next_ones_overlay() {
    let overlay = Arc::new(Mutex::new(None::<Notice>));
    let old = operation(1);
    let (saving_tx, saving_rx) = std::sync::mpsc::channel();
    let (saved_tx, saved_rx) = std::sync::mpsc::channel::<()>();
    let pipeline = {
        let overlay = Arc::clone(&overlay);
        let old = old.clone();
        std::thread::spawn(move || {
            // Its text came back empty with the microphone lost: it
            // passed the cancel check and now saves its history row.
            assert!(!old.is_cancelled());
            saving_tx.send(()).unwrap();
            saved_rx.recv().unwrap();
            hold_mic_lost_notice(&mut overlay.lock().unwrap(), &old)
        })
    };

    saving_rx.recv().unwrap();
    // The user cancels; nothing was pasted, so the coordinator is idle
    // at once and the next dictation starts.
    assert!(old.cancel());
    assert!(dictation_takes_overlay(
        &mut overlay.lock().unwrap(),
        "recording"
    ));
    saved_tx.send(()).unwrap();

    assert!(!pipeline.join().unwrap(), "the old warning is not shown");
    let mut overlay = overlay.lock().unwrap();
    assert!(overlay.is_none());
    assert!(
        dictation_takes_overlay(&mut overlay, "transcribing"),
        "the new dictation still shows its transcribing state"
    );
}

#[test]
fn a_cancel_after_the_paste_keeps_the_microphone_warning() {
    // Text already pasted cannot be taken back, so its warning stands.
    let pasted = operation(1);
    assert!(pasted.try_enter_paste().is_some());
    assert!(!pasted.cancel());
    let mut notice = None;
    assert!(hold_mic_lost_notice(&mut notice, &pasted));
    assert!(matches!(notice, Some(Notice::MicLost)));
}
