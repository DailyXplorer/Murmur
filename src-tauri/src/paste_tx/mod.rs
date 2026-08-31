//! Guarded clipboard paste.
//!
//! Restoring the previous clipboard after a fixed delay is unsafe. The paste
//! keystroke is only *enqueued* at that point — the target application reads
//! the clipboard whenever its event loop gets to it, so a delayed target can
//! receive the previous clipboard value instead (#502).
//!
//! This module instead publishes the transcript as a *lazy promise*. The
//! operating system tells us when a consumer requests its text, but does not
//! identify that consumer:
//!
//! On macOS, `declareTypes:owner:` installs an owner object and the pasteboard
//! calls `pasteboard:provideDataForType:` on read.
//!
//! Three rules keep settlement conservative:
//!
//! 1. A read is diagnostic only. It never triggers auto-submit, restoration,
//!    or early cleanup because it may come from a clipboard manager.
//! 2. Auto-submit never depends on a read. Cmd+V and Return are posted directly
//!    to the captured PID, with the full app identity revalidated before each.
//! 3. Murmur only clears or materializes its transcript while it still owns
//!    the clipboard. If the user copied something else, their action wins.
//!
//! Settlement uses a bounded lifetime that is never shortened by reads. Since
//! the callback carries no reader identity, earlier clipboard data is never
//! restored, with or without a read.

use std::time::{Duration, Instant};

mod macos;

/// Bounded lifetime of the promised transcript. Reads never shorten it because
/// AppKit does not reveal whether the intended target or a third party read.
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
    /// Stop waiting and settle the transaction without restoring prior data.
    Finish,
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

/// Attempts the guarded promised-data paste. Returns `Err` before anything has
/// been published when the macOS transaction cannot start, in which case the
/// caller uses a non-restoring fallback. On `Ok`, publishing and chord
/// injection have completed and settlement finishes asynchronously.
pub(crate) fn try_reliable_paste(
    text: &str,
    app_handle: &tauri::AppHandle,
    auto_submit: bool,
    auto_submit_key: crate::settings::AutoSubmitKey,
    clipboard_handling: crate::settings::ClipboardHandling,
) -> Result<(), String> {
    macos::run(
        text,
        app_handle,
        auto_submit,
        auto_submit_key,
        clipboard_handling,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
