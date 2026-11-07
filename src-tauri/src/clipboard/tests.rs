use super::*;
use crate::operation::OperationId;
use std::sync::atomic::{AtomicUsize, Ordering};

fn after_word(_: Instant) -> Option<Before> {
    Some(Before {
        last: 'd',
        previous: Some('r'),
        last_is_emoji: false,
    })
}

#[tokio::test]
async fn leading_space_formats_only_enabled_paste_methods() {
    let reads = Arc::new(AtomicUsize::new(0));
    let read_word = || {
        let reads = Arc::clone(&reads);
        move |deadline| {
            reads.fetch_add(1, Ordering::SeqCst);
            after_word(deadline)
        }
    };
    for method in [PasteMethod::CtrlV, PasteMethod::Direct] {
        assert_eq!(
            with_leading_space("next".into(), true, method, read_word()).await,
            " next"
        );
        assert_eq!(
            with_leading_space("next".into(), false, method, read_word()).await,
            "next"
        );
    }
    assert_eq!(
        with_leading_space("next".into(), true, PasteMethod::None, read_word()).await,
        "next"
    );
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unavailable_or_failed_caret_reads_leave_the_text_unchanged() {
    assert_eq!(
        with_leading_space("next".into(), true, PasteMethod::CtrlV, |_| None).await,
        "next"
    );
    assert_eq!(
        with_leading_space("next".into(), true, PasteMethod::Direct, |_| {
            panic!("caret reader failed")
        })
        .await,
        "next"
    );
}

#[tokio::test]
async fn a_hung_caret_read_does_not_hold_the_paste() {
    let (release, blocked) = std::sync::mpsc::channel();
    let (started, reading) = oneshot::channel();
    let paste = tokio::spawn(with_leading_space(
        "next".into(),
        true,
        PasteMethod::CtrlV,
        move |deadline| {
            let _ = started.send(());
            let _ = blocked.recv();
            after_word(deadline)
        },
    ));
    reading.await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), paste).await;
    release.send(()).unwrap();
    assert_eq!(result.unwrap().unwrap(), "next");
}

#[cfg(target_os = "macos")]
fn target(pid: i32) -> FrontmostTarget {
    FrontmostTarget::new(
        pid,
        Some(format!("com.example.{pid}")),
        Some(f64::from(pid)),
    )
    .unwrap()
}

#[cfg(target_os = "windows")]
fn target(pid: i32) -> FrontmostTarget {
    FrontmostTarget::new(pid as u32, pid as isize * 16, Some(pid as u64)).unwrap()
}

#[cfg(target_os = "windows")]
#[test]
fn a_windows_target_is_one_window_of_one_process() {
    let window = FrontmostTarget::new(42, 0x1000, Some(7));
    assert!(window.is_some());
    // Another window of the same process is another target.
    assert!(!same_frontmost_target(
        window.clone(),
        FrontmostTarget::new(42, 0x2000, Some(7))
    ));
    // So is a later process that reused the pid.
    assert!(!same_frontmost_target(
        window.clone(),
        FrontmostTarget::new(42, 0x1000, Some(8))
    ));
    // A process whose start time Windows withheld is still a target.
    assert!(FrontmostTarget::new(42, 0x1000, None).is_some());
    assert_eq!(FrontmostTarget::new(0, 0x1000, None), None);
    assert_eq!(FrontmostTarget::new(42, 0, None), None);
    assert_eq!(FrontmostTarget::new(std::process::id(), 0x1000, None), None);
}

#[cfg(target_os = "macos")]
#[test]
fn an_app_without_a_bundle_id_or_launch_date_is_still_a_paste_target() {
    let unbundled = FrontmostTarget::new(42, None, None);
    assert!(unbundled.is_some());
    assert!(same_frontmost_target(unbundled.clone(), unbundled.clone()));
    assert!(!same_frontmost_target(
        unbundled.clone(),
        FrontmostTarget::new(43, None, None)
    ));
    assert!(!same_frontmost_target(
        unbundled,
        FrontmostTarget::new(42, None, Some(1.0))
    ));
    assert_eq!(FrontmostTarget::new(0, None, None), None);
    let own_pid = i32::try_from(std::process::id()).unwrap();
    assert_eq!(FrontmostTarget::new(own_pid, None, None), None);
}

#[cfg(target_os = "macos")]
#[test]
fn output_goes_to_the_app_that_holds_the_keyboard() {
    const OWN: u32 = 7;
    // (frontmost, focused, expected)
    let cases = [
        // Spotlight's panel over Slack: the panel holds the keyboard.
        (Some(100), Some(200), Some(200)),
        (Some(100), Some(100), Some(100)),
        // No Accessibility answer, or Murmur's own window.
        (Some(100), None, Some(100)),
        (Some(100), Some(7), Some(100)),
        (Some(100), Some(0), Some(100)),
        (None, Some(200), Some(200)),
        (Some(7), None, None),
        (Some(0), None, None),
        (None, None, None),
    ];
    for (frontmost, focused, expected) in cases {
        assert_eq!(
            keyboard_target_pid(frontmost, focused, OWN),
            expected,
            "frontmost {frontmost:?}, focused {focused:?}"
        );
    }
}

#[test]
fn frontmost_target_comparison_fails_closed() {
    let first = Some(target(42));
    assert!(same_frontmost_target(first.clone(), first.clone()));
    assert!(!same_frontmost_target(first.clone(), Some(target(43))));
    assert!(!same_frontmost_target(first.clone(), None));
    assert!(!same_frontmost_target(None, first));
}

#[test]
fn clipboard_only_failure_is_reported_as_recoverable_output() {
    let operation = ProcessingOperation::new(OperationId(10));
    let outcome =
        complete_clipboard_only_output(&operation, ClipboardHandling::CopyToClipboard, || {
            Err("clipboard unavailable".to_string())
        });

    assert_eq!(outcome, PasteOutcome::Skipped);
    assert!(!operation.cancel());
}

#[test]
fn clipboard_only_success_completes_without_paste() {
    let operation = ProcessingOperation::new(OperationId(11));
    let mut wrote_text = false;
    let outcome =
        complete_clipboard_only_output(&operation, ClipboardHandling::CopyToClipboard, || {
            wrote_text = true;
            Ok(())
        });

    assert_eq!(outcome, PasteOutcome::IntentionalNone);
    assert!(wrote_text);
}

#[test]
fn clipboard_only_cancel_and_history_only_mode_never_write() {
    let cancelled = ProcessingOperation::new(OperationId(12));
    assert!(cancelled.cancel());
    assert_eq!(
        complete_clipboard_only_output(&cancelled, ClipboardHandling::CopyToClipboard, || panic!(
            "cancelled output must not touch the clipboard"
        ),),
        PasteOutcome::Cancelled
    );

    assert_eq!(
        complete_clipboard_only_output(
            &ProcessingOperation::new(OperationId(13)),
            ClipboardHandling::DontModify,
            || panic!("history-only output must not touch the clipboard"),
        ),
        PasteOutcome::IntentionalNone
    );
}

#[cfg(target_os = "macos")]
#[test]
fn without_a_text_field_the_clipboard_is_written_only_when_copy_is_clicked() {
    let clipboard = Arc::new(Mutex::new(Vec::<String>::new()));
    let pasteboard = Arc::clone(&clipboard);
    let write: ClipboardWriter = Arc::new(move |text| {
        pasteboard.lock().unwrap().push(text.to_string());
        Ok(())
    });
    let operation = ProcessingOperation::new(OperationId(14));

    let outcome = offer_copy(&operation, "hello".into(), write, |text, write| {
        crate::overlay::hold_copy_notice(text, write, false);
        true
    });
    assert_eq!(outcome, PasteOutcome::CopyOffered);
    assert!(!operation.cancel(), "the offer claimed the output");
    assert!(
        clipboard.lock().unwrap().is_empty(),
        "nothing copied before the click"
    );

    crate::overlay::copy_held_notice().unwrap();
    assert_eq!(clipboard.lock().unwrap().as_slice(), ["hello"]);
}

#[cfg(target_os = "macos")]
fn untouchable_clipboard() -> ClipboardWriter {
    Arc::new(|_| panic!("the clipboard must not be written"))
}

#[cfg(target_os = "macos")]
#[test]
fn without_the_overlay_the_offer_is_skipped_for_recovery() {
    let outcome = offer_copy(
        &ProcessingOperation::new(OperationId(15)),
        "hello".into(),
        untouchable_clipboard(),
        |_, _| false,
    );

    assert_eq!(outcome, PasteOutcome::Skipped);
}

#[cfg(target_os = "macos")]
#[test]
fn a_cancelled_offer_never_shows() {
    let cancelled = ProcessingOperation::new(OperationId(16));
    assert!(cancelled.cancel());

    assert_eq!(
        offer_copy(
            &cancelled,
            "hello".into(),
            untouchable_clipboard(),
            |_, _| panic!("cancelled output must not show the copy pill")
        ),
        PasteOutcome::Cancelled
    );
}

#[test]
fn late_cancel_keeps_the_paste_but_marks_auto_submit_ineligible() {
    let operation = ProcessingOperation::new(OperationId(1));
    let _permit = operation
        .try_enter_paste()
        .expect("paste should claim output");

    assert!(!operation.cancel());
    assert!(!operation.is_cancelled());
    assert!(operation.cancellation_requested());
}

#[test]
fn fake_main_queue_releases_before_late_cancel_can_reach_auto_submit() {
    type MainCallback = Box<dyn FnOnce() + Send>;

    struct FakeMainQueue {
        callbacks: Arc<Mutex<Vec<MainCallback>>>,
    }

    impl FakeMainQueue {
        fn enqueue(&self, callback: MainCallback) {
            self.callbacks.lock().unwrap().push(callback);
        }

        fn run_all(&self) {
            let callbacks = std::mem::take(&mut *self.callbacks.lock().unwrap());
            for callback in callbacks {
                callback();
            }
        }
    }

    let queue = FakeMainQueue {
        callbacks: Arc::new(Mutex::new(Vec::new())),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let (scheduled, scheduled_receipt) = oneshot::channel();
    let queue_for_dispatch = queue.callbacks.clone();
    let events_for_release = events.clone();
    let release = schedule_paste_modifier_release_with(
        Duration::ZERO,
        42,
        move |callback| {
            FakeMainQueue {
                callbacks: queue_for_dispatch,
            }
            .enqueue(callback);
            let _ = scheduled.send(());
            Ok(())
        },
        move |_| {
            events_for_release.lock().unwrap().push("release");
            Ok(())
        },
    );

    tauri::async_runtime::block_on(async { scheduled_receipt.await.unwrap() });
    let operation = ProcessingOperation::new(OperationId(2));
    let _permit = operation.try_enter_paste().unwrap();
    assert!(!operation.cancel());

    queue.run_all();
    tauri::async_runtime::block_on(release.wait()).unwrap();
    if !operation.cancellation_requested() {
        events.lock().unwrap().push("return");
    }
    assert_eq!(events.lock().unwrap().as_slice(), &["release"]);
}

#[test]
fn clipboard_source_has_no_blocking_thread_sleep() {
    let forbidden = ["thread", "::sleep"].concat();
    assert!(!include_str!("../clipboard.rs").contains(&forbidden));
}
