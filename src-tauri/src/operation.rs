//! Identity and cancellation state for one transcription pipeline.
//!
//! An operation has one irreversible output boundary. Cancellation and paste
//! claim that boundary atomically so a cancelled operation cannot later write
//! a clipboard value or history row belonging to a newer recording.

use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc,
};
use tokio::sync::watch;

const ACTIVE: u8 = 0;
const CANCELLED: u8 = 1;
const OUTPUT_CLAIMED: u8 = 2;

/// Stable identity assigned by the coordinator when recording becomes work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct OperationId(pub u64);

struct OperationState {
    phase: AtomicU8,
    cancellation_requested: AtomicBool,
    cancelled_tx: watch::Sender<bool>,
}

/// Shared lifecycle state for one transcription operation.
///
/// Clones identify the same operation. `cancel` and `try_enter_paste` are the
/// only phase transitions and race atomically.
#[derive(Clone)]
pub struct ProcessingOperation {
    id: OperationId,
    state: Arc<OperationState>,
}

/// Proof that this operation reached its irreversible output boundary first.
///
/// This is deliberately non-cloneable. Once issued, cancellation is too late
/// to retract a targeted key event or pasteboard publication.
pub struct PastePermit {
    operation: ProcessingOperation,
}

impl ProcessingOperation {
    pub(crate) fn new(id: OperationId) -> Self {
        let (cancelled_tx, _) = watch::channel(false);
        Self {
            id,
            state: Arc::new(OperationState {
                phase: AtomicU8::new(ACTIVE),
                cancellation_requested: AtomicBool::new(false),
                cancelled_tx,
            }),
        }
    }

    pub fn id(&self) -> OperationId {
        self.id
    }

    /// Requests cancellation and cancels while output is still reversible.
    ///
    /// The request is retained even when it arrives after output was claimed:
    /// a posted paste chord cannot be retracted, but delayed auto-submit and
    /// provider work that has not crossed its own boundary must still stop.
    /// Returns false once paste has been claimed.
    pub fn cancel(&self) -> bool {
        self.state
            .cancellation_requested
            .store(true, Ordering::Release);
        let result = match self.state.phase.compare_exchange(
            ACTIVE,
            CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(CANCELLED) => true,
            Err(_) => false,
        };
        // Keep the cancellation state for subscribers created after this
        // transition too. `send` drops the value when there are no receivers
        // yet, which can otherwise strand a later provider future forever.
        self.state.cancelled_tx.send_replace(true);
        result
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.phase.load(Ordering::Acquire) == CANCELLED
    }

    /// True for any cancellation request, including one that arrived after an
    /// irreversible paste boundary. Native paste transactions must retain their
    /// promised data/settlement in that case, but they must not auto-submit.
    pub fn cancellation_requested(&self) -> bool {
        self.state.cancellation_requested.load(Ordering::Acquire)
    }

    /// Wait for cancellation without polling. Provider implementations select
    /// this against their real network I/O so dropping a task is not mistaken
    /// for cancelling its transport.
    pub(crate) async fn cancelled(&self) {
        let mut cancelled = self.state.cancelled_tx.subscribe();
        if *cancelled.borrow() {
            return;
        }
        while cancelled.changed().await.is_ok() {
            if *cancelled.borrow() {
                return;
            }
        }
    }

    pub(crate) fn subscribe_cancel(&self) -> watch::Receiver<bool> {
        self.state.cancelled_tx.subscribe()
    }

    /// Claims the paste/history boundary if cancellation has not won already.
    /// The caller must invoke this at the actual publish/injection point, not
    /// while merely scheduling a delayed paste.
    pub fn try_enter_paste(&self) -> Option<PastePermit> {
        self.state
            .phase
            .compare_exchange(ACTIVE, OUTPUT_CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| PastePermit {
                operation: self.clone(),
            })
    }
}

impl PastePermit {
    pub fn operation(&self) -> &ProcessingOperation {
        &self.operation
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationId, ProcessingOperation};

    #[test]
    fn cancellation_before_paste_prevents_output_claim() {
        let operation = ProcessingOperation::new(OperationId(1));
        assert!(operation.cancel());
        assert!(operation.is_cancelled());
        assert!(operation.try_enter_paste().is_none());
    }

    #[test]
    fn claimed_paste_cannot_be_cancelled_afterward() {
        let operation = ProcessingOperation::new(OperationId(1));
        let permit = operation.try_enter_paste();
        assert!(permit.is_some());
        assert!(!operation.cancel());
        assert!(!operation.is_cancelled());
        assert!(operation.cancellation_requested());
    }

    #[test]
    fn cancellation_notifies_waiters() {
        let operation = ProcessingOperation::new(OperationId(1));
        let wait = operation.cancelled();
        assert!(operation.cancel());
        tauri::async_runtime::block_on(wait);
    }
}
