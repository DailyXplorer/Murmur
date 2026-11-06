//! Guarded clipboard paste.
//!
//! Restoring the previous clipboard after a fixed delay is unsafe. The paste
//! keystroke is only *enqueued* at that point. The target application reads
//! the clipboard whenever its event loop gets to it, so a delayed target can
//! receive the previous clipboard value instead.
//!
//! This module instead publishes the transcript as a *lazy promise*. The
//! operating system tells us when a consumer requests its text, but does not
//! identify that consumer:
//!
//! On macOS, `declareTypes:owner:` installs an owner object and the pasteboard
//! calls `pasteboard:provideDataForType:` on read. On Windows, delayed
//! rendering (`SetClipboardData` with no data) makes the system send the
//! owner window `WM_RENDERFORMAT` on read.
//!
//! Three rules keep settlement conservative:
//!
//! 1. A read is diagnostic only. It never triggers auto-submit, restoration,
//!    or early cleanup because it may come from a clipboard manager.
//! 2. Auto-submit never depends on a read. Cmd+V and Return are posted directly
//!    to the captured PID, with the full app identity revalidated before each.
//! 3. Murmur only restores or materializes anything while it still owns the
//!    clipboard. If the user copied something else, their action wins.
//!
//! Settlement uses a bounded lifetime that is never shortened by reads, so a
//! slow target has had the full window to read the transcript before the
//! user's previous clipboard (every item and type, saved before publishing)
//! is put back.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::operation::ProcessingOperation;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use self::windows as platform;
pub(crate) use platform::ReliablePasteReceipt;

/// Bounded lifetime of the promised transcript. Reads never shorten it because
/// the system does not reveal whether the intended target or a third party read.
pub(crate) const SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(8);

/// When the chord could not be injected at all, no legitimate receipt can
/// arrive, so retain the transcript quickly instead of waiting the full time.
pub(crate) const FAILED_INJECTION_TIMEOUT: Duration = Duration::from_millis(500);

/// Shared, cross-thread record of one paste transaction.
#[derive(Debug)]
pub(crate) struct TxState {
    /// When the transcript was published to the clipboard.
    pub published_at: Instant,
    /// When the paste chord was injected. Reads after this point are logged,
    /// but never treated as proof that the target consumed the transcript.
    pub injected_at: Option<Instant>,
    /// The chord could not be sent; short-circuit the wait.
    pub injection_failed: bool,
    /// Someone else took clipboard ownership (user copied elsewhere, ...).
    pub ownership_lost: bool,
    /// A newer paste transaction settled this one early.
    pub cancelled: bool,
    /// First post-injection read has been logged for diagnostics.
    pub logged_receipt: bool,
}

impl TxState {
    pub fn new() -> Self {
        Self {
            published_at: Instant::now(),
            injected_at: None,
            injection_failed: false,
            ownership_lost: false,
            cancelled: false,
            logged_receipt: false,
        }
    }

    /// Logs the first post-injection read. Reads are never security evidence.
    pub fn record_receipt(&mut self, at: Instant) {
        if !self.logged_receipt {
            if let Some(injected) = self.injected_at {
                if at >= injected {
                    self.logged_receipt = true;
                    log::info!(
                        "[reliable-paste] clipboard read {}ms after chord",
                        at.duration_since(injected).as_millis()
                    );
                }
            }
        }
    }
}

pub(crate) enum WaitDecision {
    KeepWaiting,
    /// Stop waiting and settle the transaction.
    Finish,
}

/// Result of attempting the initial guarded-paste transaction. A successful
/// return always means the transaction owns its later settlement; callers must
/// not fall back and overwrite its recovery text.
pub(crate) enum ReliablePasteOutcome {
    Cancelled,
    Injected(ReliablePasteReceipt),
    Skipped,
}

/// Pure decision: keep the promise alive for its bounded lifetime, or finish.
pub(crate) fn evaluate(state: &TxState, now: Instant) -> WaitDecision {
    if state.ownership_lost || state.cancelled {
        return WaitDecision::Finish;
    }
    let deadline = if state.injection_failed {
        FAILED_INJECTION_TIMEOUT
    } else {
        SETTLEMENT_TIMEOUT
    };
    if now.duration_since(state.published_at) >= deadline {
        return WaitDecision::Finish;
    }
    WaitDecision::KeepWaiting
}

/// Limits on saving the previous clipboard. Reading a format the source app
/// provided lazily makes that app render it synchronously, right before the
/// paste, so the save must stay small and quick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptureBudget {
    pub max_bytes: usize,
    pub max_duration: Duration,
}

pub(crate) const CAPTURE_BUDGET: CaptureBudget = CaptureBudget {
    max_bytes: 32 * 1024 * 1024,
    max_duration: Duration::from_millis(150),
};

/// Why the previous clipboard was not saved. Settlement then erases the
/// transcript instead of restoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureLimit {
    Bytes,
    Time,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClipboardSettlement {
    RestorePrevious,
    KeepTranscript,
    LeaveUntouched,
}

pub(crate) fn decide_settlement(
    still_ours: bool,
    preserve_transcript: bool,
) -> ClipboardSettlement {
    if !still_ours {
        ClipboardSettlement::LeaveUntouched
    } else if preserve_transcript {
        ClipboardSettlement::KeepTranscript
    } else {
        ClipboardSettlement::RestorePrevious
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum PasteboardAdmission {
    Claimed,
    Cancelled,
    TargetChanged,
}

/// Rejected callbacks must not settle an older transaction or publish text.
pub(crate) fn claim_before_pasteboard_mutation<F>(
    operation: &ProcessingOperation,
    target_is_current: impl FnOnce() -> bool,
    mutation: F,
) -> PasteboardAdmission
where
    F: FnOnce(),
{
    if !target_is_current() {
        return PasteboardAdmission::TargetChanged;
    }
    let Some(_permit) = operation.try_enter_paste() else {
        return PasteboardAdmission::Cancelled;
    };
    mutation();
    PasteboardAdmission::Claimed
}

pub(crate) fn same_pending<T>(current: Option<&Arc<T>>, candidate: &Arc<T>) -> bool {
    current
        .map(|pending| Arc::ptr_eq(pending, candidate))
        .unwrap_or(false)
}

/// Settles a paste still inside its bounded lifetime. The promised data dies
/// with the process and the saved clipboard lives only in its memory, so
/// quitting without this leaves the user's copied content lost; a clipboard
/// write that must stay must also come after it, or the settlement could put
/// the previous clipboard back over it. Must run on the main thread.
pub(crate) fn settle_pending() {
    platform::flush_pending();
}

/// Attempts the guarded promised-data paste. A `Skipped` result means the
/// operation won the output boundary but no chord was injected, so callers
/// must not start a second paste. On `Injected(_)`, settlement finishes
/// asynchronously while the receipt covers the cancellable auto-submit delay.
pub(crate) fn try_reliable_paste(
    text: &str,
    app_handle: &tauri::AppHandle,
    auto_submit: bool,
    auto_submit_key: crate::settings::AutoSubmitKey,
    clipboard_handling: crate::settings::ClipboardHandling,
    operation: ProcessingOperation,
) -> ReliablePasteOutcome {
    platform::run(
        text,
        app_handle,
        auto_submit,
        auto_submit_key,
        clipboard_handling,
        operation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn state_after_publish(published_ago: Duration) -> TxState {
        let mut s = TxState::new();
        s.published_at = Instant::now() - published_ago;
        s
    }

    #[test]
    fn keeps_waiting_without_receipt_within_timeout() {
        let s = state_after_publish(Duration::from_millis(100));
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn read_never_shortens_settlement_lifetime() {
        let mut s = state_after_publish(Duration::from_millis(300));
        s.injected_at = Some(Instant::now() - Duration::from_millis(250));
        s.record_receipt(Instant::now() - Duration::from_millis(200));
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn pre_injection_receipt_does_not_count() {
        let mut s = state_after_publish(Duration::from_millis(300));
        s.record_receipt(Instant::now() - Duration::from_millis(200));
        s.injected_at = Some(Instant::now() - Duration::from_millis(100));
        assert!(!s.logged_receipt);
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn finishes_on_timeout_without_receipt() {
        let s = state_after_publish(SETTLEMENT_TIMEOUT);
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn failed_injection_uses_short_timeout() {
        let mut s = state_after_publish(FAILED_INJECTION_TIMEOUT);
        s.injection_failed = true;
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn ownership_loss_finishes_immediately() {
        let mut s = state_after_publish(Duration::from_millis(10));
        s.ownership_lost = true;
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn restores_the_previous_clipboard_only_while_murmur_still_owns_it() {
        assert_eq!(
            decide_settlement(true, false),
            ClipboardSettlement::RestorePrevious
        );
        assert_eq!(
            decide_settlement(false, false),
            ClipboardSettlement::LeaveUntouched
        );
    }

    #[test]
    fn copy_to_clipboard_always_keeps_transcript() {
        assert_eq!(
            decide_settlement(true, true),
            ClipboardSettlement::KeepTranscript
        );
    }

    #[test]
    fn failed_injection_keeps_transcript_for_recovery() {
        let preserve_transcript = false;
        let injection_failed = true;
        assert_eq!(
            decide_settlement(true, preserve_transcript || injection_failed),
            ClipboardSettlement::KeepTranscript
        );
    }

    #[test]
    fn cancellation_before_queued_callback_leaves_live_pending_unsettled() {
        use crate::operation::OperationId;

        let operation = ProcessingOperation::new(OperationId(7));
        let pending = Arc::new(Mutex::new(Some("live-pr64-transaction")));
        assert!(operation.cancel());

        let pending_for_mutation = pending.clone();
        assert_eq!(
            claim_before_pasteboard_mutation(
                &operation,
                || true,
                move || {
                    pending_for_mutation.lock().unwrap().take();
                }
            ),
            PasteboardAdmission::Cancelled
        );
        assert_eq!(*pending.lock().unwrap(), Some("live-pr64-transaction"));
    }

    #[test]
    fn target_changed_before_publication_leaves_clipboard_and_pending_untouched() {
        use crate::operation::OperationId;

        let operation = ProcessingOperation::new(OperationId(8));
        let mut pending = Some("previous-transaction");
        let mut clipboard = "original-clipboard";
        let captured_target = "application-a";
        let current_target = "application-b";

        let admission = claim_before_pasteboard_mutation(
            &operation,
            || current_target == captured_target,
            || {
                pending.take();
                clipboard = "transcript";
            },
        );

        assert_eq!(admission, PasteboardAdmission::TargetChanged);
        assert_eq!(pending, Some("previous-transaction"));
        assert_eq!(clipboard, "original-clipboard");
        assert!(operation.try_enter_paste().is_some());
    }

    #[test]
    fn auto_submit_guard_accepts_only_the_current_transaction() {
        fn pending() -> Arc<Mutex<()>> {
            Arc::new(Mutex::new(()))
        }

        let current = pending();
        let superseded = pending();

        assert!(same_pending(Some(&current), &current));
        assert!(!same_pending(Some(&current), &superseded));
        assert!(!same_pending(None, &current));
    }
}
