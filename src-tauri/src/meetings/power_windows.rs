//! Keeps the PC from sleeping on its own while a meeting records. Closing the
//! lid, or a battery running out, still puts it to sleep.
//!
//! The execution state belongs to the thread that sets it, so the guard owns
//! a thread that sets it, waits, and clears it.

use std::sync::mpsc;
use std::thread::JoinHandle;

use log::warn;
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED, EXECUTION_STATE,
};

/// Holds `ES_SYSTEM_REQUIRED` until dropped. The display still turns off, and
/// Windows drops the request if Murmur exits.
pub struct AwakeGuard {
    release: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

fn set_execution_state(state: EXECUTION_STATE) -> bool {
    // SAFETY: a plain call that changes the calling thread's state only. It
    // returns the previous state, which is 0 only on failure.
    unsafe { SetThreadExecutionState(state) }.0 != 0
}

impl AwakeGuard {
    /// `_reason` is what the macOS build shows in `pmset`; Windows takes none
    /// for this call.
    pub fn hold(_reason: &str) -> Option<Self> {
        Self::hold_with(set_execution_state)
    }

    /// Holds the state through `set`, which reports whether Windows took it.
    fn hold_with(set: impl Fn(EXECUTION_STATE) -> bool + Send + 'static) -> Option<Self> {
        let (release, released) = mpsc::channel::<()>();
        let (started, taken) = mpsc::channel::<bool>();
        let thread = std::thread::Builder::new()
            .name("meeting-keep-awake".into())
            .spawn(move || {
                let held = set(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
                let _ = started.send(held);
                if held {
                    // Returns when the guard drops its sender.
                    let _ = released.recv();
                    set(ES_CONTINUOUS);
                }
            })
            .ok()?;
        if taken.recv() == Ok(true) {
            Some(Self {
                release: Some(release),
                thread: Some(thread),
            })
        } else {
            let _ = thread.join();
            warn!("Windows refused to keep the PC awake");
            None
        }
    }
}

impl Drop for AwakeGuard {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn recorder(accept: bool) -> (Arc<Mutex<Vec<u32>>>, impl Fn(EXECUTION_STATE) -> bool) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&calls);
        (calls, move |state: EXECUTION_STATE| {
            seen.lock().unwrap().push(state.0);
            accept
        })
    }

    #[test]
    fn the_pc_is_kept_awake_until_the_guard_is_dropped() {
        let (calls, set) = recorder(true);
        let guard = AwakeGuard::hold_with(set).unwrap();
        let held = ES_CONTINUOUS.0 | ES_SYSTEM_REQUIRED.0;
        assert_eq!(*calls.lock().unwrap(), vec![held]);
        drop(guard);
        assert_eq!(*calls.lock().unwrap(), vec![held, ES_CONTINUOUS.0]);
    }

    #[test]
    fn a_refused_request_gives_no_guard_and_clears_nothing() {
        let (calls, set) = recorder(false);
        assert!(AwakeGuard::hold_with(set).is_none());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![ES_CONTINUOUS.0 | ES_SYSTEM_REQUIRED.0]
        );
    }

    #[test]
    fn windows_takes_the_request_and_lets_go() {
        let guard = AwakeGuard::hold("Murmur is recording a meeting").unwrap();
        drop(guard);
    }
}
