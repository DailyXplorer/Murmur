//! Serializes a complete load/change/commit, not just individual store calls.
//! Keep callbacks free of application locks, events and native side effects.

use std::sync::{Mutex, PoisonError};

pub(crate) struct Transactions(Mutex<()>);

impl Transactions {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    #[cfg(test)]
    pub(crate) fn is_locked(&self) -> bool {
        matches!(self.0.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }

    /// Reads also take the lock because loading may repair stored values.
    pub(crate) fn read<T>(&self, load: impl FnOnce() -> T) -> T {
        let _transaction = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        load()
    }

    pub(crate) fn update<S, T, E>(
        &self,
        load: impl FnOnce() -> Result<S, E>,
        change: impl FnOnce(&mut S) -> Result<T, E>,
        commit: impl FnOnce(S) -> Result<(), E>,
    ) -> Result<T, E> {
        let _transaction = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut settings = load()?;
        let result = change(&mut settings)?;
        commit(settings)?;
        Ok(result)
    }
}

/// The checked commit used by the real settings store. The caller keeps its
/// transaction lock until both saving and any failed-save rollback finish.
pub(super) fn persist_checked<V, E: std::fmt::Display>(
    next: V,
    load: impl FnOnce() -> Option<V>,
    replace: impl Fn(Option<V>),
    save: impl Fn() -> Result<(), E>,
) -> Result<(), String> {
    let previous = load();
    replace(Some(next));
    if let Err(write_error) = save() {
        replace(previous);
        return match save() {
            Ok(()) => Err(format!("Failed to persist settings: {write_error}")),
            Err(restore_error) => Err(format!(
                "Failed to persist settings: {write_error}. Failed to restore the previous settings: {restore_error}"
            )),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, TryLockError};
    use std::thread;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(5);

    // Each store call has its own lock, just like the plugin; only the
    // production Transactions boundary makes read-modify-write atomic.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Settings {
        words: Vec<&'static str>,
        context: bool,
    }

    fn initial() -> Settings {
        Settings {
            words: vec![],
            context: true,
        }
    }

    #[test]
    fn independent_updates_cannot_enter_between_load_and_commit() {
        let transactions = Arc::new(Transactions::new());
        let store = Arc::new(Mutex::new(initial()));
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let learner = {
            let transactions = Arc::clone(&transactions);
            let store = Arc::clone(&store);
            thread::spawn(move || {
                transactions.update(
                    || {
                        assert!(
                            transactions.is_locked(),
                            "lock before loading, not only before writing"
                        );
                        Ok::<_, ()>(store.lock().unwrap().clone())
                    },
                    |settings| {
                        loaded_tx.send(()).unwrap();
                        resume_rx.recv_timeout(TIMEOUT).unwrap();
                        settings.words.push("Murmur");
                        Ok(())
                    },
                    |settings| {
                        *store.lock().unwrap() = settings;
                        Ok(())
                    },
                )
            })
        };
        loaded_rx.recv_timeout(TIMEOUT).unwrap();
        // A deterministic assertion, not a sleep hoping the second thread
        // was scheduled. Removing the transaction lock fails right here.
        assert!(transactions.is_locked());
        let toggle = {
            let transactions = Arc::clone(&transactions);
            let store = Arc::clone(&store);
            thread::spawn(move || {
                transactions.update(
                    || Ok::<_, ()>(store.lock().unwrap().clone()),
                    |settings| {
                        settings.context = false;
                        Ok(())
                    },
                    |settings| {
                        *store.lock().unwrap() = settings;
                        Ok(())
                    },
                )
            })
        };
        resume_tx.send(()).unwrap();
        learner.join().unwrap().unwrap();
        toggle.join().unwrap().unwrap();
        assert_eq!(
            *store.lock().unwrap(),
            Settings {
                words: vec!["Murmur"],
                context: false,
            }
        );
    }

    #[test]
    fn a_repairing_read_holds_the_same_transaction_lock() {
        let transactions = Transactions::new();
        transactions.read(|| {
            assert!(matches!(
                transactions.0.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
        });
    }

    #[test]
    fn failed_mutation_is_not_committed() {
        let transactions = Transactions::new();
        let store = Mutex::new(initial());
        let result = transactions.update(
            || Ok(store.lock().unwrap().clone()),
            |settings| {
                settings.context = false;
                Err::<(), _>("invalid change")
            },
            |settings| {
                *store.lock().unwrap() = settings;
                Ok(())
            },
        );
        assert_eq!(result, Err("invalid change"));
        assert_eq!(*store.lock().unwrap(), initial());
    }

    #[test]
    fn failed_commit_keeps_rollback_inside_the_transaction() {
        let transactions = Transactions::new();
        let store = Mutex::new(initial());
        let result = transactions.update(
            || Ok::<_, String>(store.lock().unwrap().clone()),
            |settings| {
                settings.context = false;
                Ok(())
            },
            |settings| {
                let saves = std::cell::Cell::new(0);
                persist_checked(
                    settings,
                    || Some(store.lock().unwrap().clone()),
                    |value| *store.lock().unwrap() = value.unwrap(),
                    || {
                        assert!(matches!(
                            transactions.0.try_lock(),
                            Err(TryLockError::WouldBlock)
                        ));
                        saves.set(saves.get() + 1);
                        if saves.get() == 1 {
                            Err("disk full")
                        } else {
                            Ok(())
                        }
                    },
                )
            },
        );
        assert_eq!(
            result,
            Err("Failed to persist settings: disk full".to_string())
        );
        assert_eq!(*store.lock().unwrap(), initial());
    }
}
