use std::collections::VecDeque;
use std::sync::Mutex;

/// An action forwarded by a second Murmur process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SingleInstanceAction {
    Show,
    Toggle,
    Cancel,
}

#[derive(Default)]
struct QueueState {
    ready: bool,
    draining: bool,
    pending: VecDeque<SingleInstanceAction>,
}

/// Holds single-instance actions until the desktop runtime is ready to handle
/// them. The readiness check and enqueue share one mutex, so an action is
/// either drained once or returned for immediate dispatch, never both.
#[derive(Default)]
pub(crate) struct SingleInstanceActionQueue {
    state: Mutex<QueueState>,
}

impl SingleInstanceActionQueue {
    /// Queues `action` while startup is in progress or while the startup batch
    /// is draining. Once the batch is complete, returns it for dispatch outside
    /// the mutex.
    pub(crate) fn enqueue_or_dispatch(
        &self,
        action: SingleInstanceAction,
    ) -> Option<SingleInstanceAction> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| {
            log::error!("Single-instance startup action queue was poisoned");
            poisoned.into_inner()
        });

        if state.ready && !state.draining {
            Some(action)
        } else {
            state.pending.push_back(action);
            None
        }
    }

    /// Marks startup complete and dispatches queued actions in FIFO order.
    /// The dispatcher always runs after releasing the mutex. Actions received
    /// while it runs stay queued behind the current batch, preventing a direct
    /// callback from overtaking an earlier startup action. Repeated calls do
    /// nothing, so only the first caller owns the drain.
    pub(crate) fn mark_ready_and_drain<F>(&self, mut dispatch: F)
    where
        F: FnMut(SingleInstanceAction),
    {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| {
            log::error!("Single-instance startup action queue was poisoned");
            poisoned.into_inner()
        });

        if state.ready {
            return;
        }

        state.ready = true;
        state.draining = true;
        drop(state);

        loop {
            let action = {
                let mut state = self.state.lock().unwrap_or_else(|poisoned| {
                    log::error!("Single-instance startup action queue was poisoned");
                    poisoned.into_inner()
                });

                match state.pending.pop_front() {
                    Some(action) => Some(action),
                    None => {
                        state.draining = false;
                        None
                    }
                }
            };

            let Some(action) = action else {
                return;
            };
            dispatch(action);
        }
    }
}

/// Preserve the existing remote-control precedence: toggle wins over cancel,
/// and ordinary second launches show the settings window.
pub(crate) fn action_from_args(args: &[String]) -> SingleInstanceAction {
    if args.iter().any(|arg| arg == "--toggle-transcription") {
        SingleInstanceAction::Toggle
    } else if args.iter().any(|arg| arg == "--cancel") {
        SingleInstanceAction::Cancel
    } else {
        SingleInstanceAction::Show
    }
}

#[cfg(test)]
mod tests {
    use super::{action_from_args, SingleInstanceAction, SingleInstanceActionQueue};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn actions_received_before_ready_drain_in_fifo_order() {
        let queue = SingleInstanceActionQueue::default();

        assert_eq!(queue.enqueue_or_dispatch(SingleInstanceAction::Show), None);
        assert_eq!(
            queue.enqueue_or_dispatch(SingleInstanceAction::Toggle),
            None
        );
        assert_eq!(
            queue.enqueue_or_dispatch(SingleInstanceAction::Cancel),
            None
        );

        let mut drained = Vec::new();
        queue.mark_ready_and_drain(|action| drained.push(action));
        assert_eq!(
            drained,
            vec![
                SingleInstanceAction::Show,
                SingleInstanceAction::Toggle,
                SingleInstanceAction::Cancel,
            ]
        );
    }

    #[test]
    fn actions_received_after_ready_dispatch_directly() {
        let queue = SingleInstanceActionQueue::default();

        queue.mark_ready_and_drain(|_| unreachable!("the queue starts empty"));
        assert_eq!(
            queue.enqueue_or_dispatch(SingleInstanceAction::Toggle),
            Some(SingleInstanceAction::Toggle)
        );
    }

    #[test]
    fn concurrent_action_and_startup_drain_are_neither_lost_nor_duplicated() {
        let queue = Arc::new(SingleInstanceActionQueue::default());
        assert_eq!(queue.enqueue_or_dispatch(SingleInstanceAction::Show), None);

        let barrier = Arc::new(Barrier::new(2));
        let (submitted_tx, submitted_rx) = std::sync::mpsc::channel();
        let queue_for_callback = Arc::clone(&queue);
        let barrier_for_callback = Arc::clone(&barrier);
        let callback = thread::spawn(move || {
            barrier_for_callback.wait();
            submitted_tx
                .send(queue_for_callback.enqueue_or_dispatch(SingleInstanceAction::Toggle))
                .unwrap();
        });

        let mut observed = Vec::new();
        queue.mark_ready_and_drain(|action| {
            observed.push(action);
            if action == SingleInstanceAction::Show {
                barrier.wait();
                assert_eq!(submitted_rx.recv().unwrap(), None);
            }
        });
        callback.join().unwrap();
        assert_eq!(
            observed,
            vec![SingleInstanceAction::Show, SingleInstanceAction::Toggle]
        );
        queue.mark_ready_and_drain(|_| panic!("the startup queue drains only once"));
    }

    #[test]
    fn startup_drain_runs_once_without_losing_show() {
        let queue = SingleInstanceActionQueue::default();
        assert_eq!(queue.enqueue_or_dispatch(SingleInstanceAction::Show), None);

        let mut drained = Vec::new();
        queue.mark_ready_and_drain(|action| drained.push(action));
        assert_eq!(drained, vec![SingleInstanceAction::Show]);
        queue.mark_ready_and_drain(|_| panic!("the startup queue drains only once"));
        assert_eq!(
            queue.enqueue_or_dispatch(SingleInstanceAction::Show),
            Some(SingleInstanceAction::Show)
        );
    }

    #[test]
    fn queue_operations_do_not_panic_during_normal_startup() {
        let result = std::panic::catch_unwind(|| {
            let queue = SingleInstanceActionQueue::default();
            assert_eq!(
                queue.enqueue_or_dispatch(SingleInstanceAction::Cancel),
                None
            );
            let mut drained = Vec::new();
            queue.mark_ready_and_drain(|action| drained.push(action));
            assert_eq!(drained, vec![SingleInstanceAction::Cancel]);
        });

        assert!(result.is_ok());
    }

    #[test]
    fn remote_argument_mapping_preserves_existing_precedence() {
        assert_eq!(action_from_args(&[]), SingleInstanceAction::Show);
        assert_eq!(
            action_from_args(&["--cancel".to_string()]),
            SingleInstanceAction::Cancel
        );
        assert_eq!(
            action_from_args(&["--cancel".to_string(), "--toggle-transcription".to_string(),]),
            SingleInstanceAction::Toggle
        );
    }
}
