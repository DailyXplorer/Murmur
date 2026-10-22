//! The one update status the tray and the settings footer show, and the
//! rule that only one check runs at a time.

use log::info;
use std::future::Future;
use std::sync::Mutex;

/// The newer version a check found, `None` when Murmur is up to date.
pub type Found = Result<Option<String>, String>;

type Publish = Box<dyn Fn(Option<String>) + Send + Sync>;

pub struct Checks {
    available: Mutex<Option<String>>,
    running: tokio::sync::Mutex<()>,
    /// Shows a changed status everywhere it appears.
    publish: Publish,
}

impl Checks {
    pub fn new(publish: impl Fn(Option<String>) + Send + Sync + 'static) -> Self {
        Self {
            available: Mutex::new(None),
            running: tokio::sync::Mutex::new(()),
            publish: Box::new(publish),
        }
    }

    pub fn available(&self) -> Option<String> {
        self.available.lock().unwrap().clone()
    }

    fn record(&self, found: Option<String>) {
        let mut available = self.available.lock().unwrap();
        if *available == found {
            return;
        }
        available.clone_from(&found);
        drop(available);
        (self.publish)(found);
    }

    /// An automatic check. It is skipped while another check runs, and a
    /// failure only goes to the log.
    pub async fn check_quietly(&self, check: impl Future<Output = Found>) {
        let Ok(_running) = self.running.try_lock() else {
            return;
        };
        match check.await {
            Ok(found) => self.record(found),
            Err(error) => info!("Automatic update check failed: {error}"),
        }
    }

    /// A check the user asked for. It waits for a running check to finish
    /// instead of running beside it.
    pub async fn check_now(&self, check: impl Future<Output = Found>) -> Found {
        let _running = self.running.lock().await;
        let found = check.await?;
        self.record(found.clone());
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::oneshot;

    fn published() -> (Checks, Arc<Mutex<Vec<Option<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let checks = Checks::new(move |found| sink.lock().unwrap().push(found));
        (checks, seen)
    }

    fn version(version: &str) -> Option<String> {
        Some(version.to_string())
    }

    #[tokio::test]
    async fn a_found_update_reaches_every_reader_once() {
        let (checks, seen) = published();

        checks.check_quietly(async { Ok(version("1.9.0")) }).await;
        checks.check_quietly(async { Ok(version("1.9.0")) }).await;

        assert_eq!(checks.available(), version("1.9.0"));
        assert_eq!(*seen.lock().unwrap(), vec![version("1.9.0")]);
    }

    #[tokio::test]
    async fn installing_elsewhere_clears_the_update() {
        let (checks, seen) = published();
        checks.check_quietly(async { Ok(version("1.9.0")) }).await;

        assert_eq!(checks.check_now(async { Ok(None) }).await, Ok(None));

        assert_eq!(checks.available(), None);
        assert_eq!(*seen.lock().unwrap(), vec![version("1.9.0"), None]);
    }

    #[tokio::test]
    async fn an_automatic_failure_is_silent_and_keeps_the_update() {
        let (checks, seen) = published();
        checks.check_quietly(async { Ok(version("1.9.0")) }).await;

        checks
            .check_quietly(async { Err("offline".to_string()) })
            .await;

        assert_eq!(checks.available(), version("1.9.0"));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_manual_failure_reaches_the_user() {
        let (checks, _) = published();

        let found = checks.check_now(async { Err("offline".to_string()) }).await;

        assert_eq!(found, Err("offline".to_string()));
    }

    #[tokio::test]
    async fn checks_never_overlap() {
        let (checks, _) = published();
        let checks = Arc::new(checks);
        let (running, max_running) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let check = |release: Option<oneshot::Receiver<()>>| {
            let (running, max_running) = (running.clone(), max_running.clone());
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                max_running.fetch_max(now, Ordering::SeqCst);
                if let Some(release) = release {
                    let _ = release.await;
                }
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(version("1.9.0"))
            }
        };

        let (release, held) = oneshot::channel();
        let first = tokio::spawn({
            let checks = checks.clone();
            let check = check(Some(held));
            async move { checks.check_quietly(check).await }
        });
        tokio::task::yield_now().await;
        assert_eq!(running.load(Ordering::SeqCst), 1, "the first check runs");

        checks.check_quietly(check(None)).await;
        let manual = tokio::spawn({
            let checks = checks.clone();
            let check = check(None);
            async move { checks.check_now(check).await }
        });
        tokio::task::yield_now().await;
        assert_eq!(running.load(Ordering::SeqCst), 1, "the manual check waits");

        release.send(()).unwrap();
        first.await.unwrap();
        assert_eq!(manual.await.unwrap(), Ok(version("1.9.0")));
        assert_eq!(max_running.load(Ordering::SeqCst), 1);
    }
}
