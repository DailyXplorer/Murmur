//! Keeps one local model loaded between dictations: loaded when a recording
//! starts, so the speaking time hides the load, and released after
//! `IDLE_RELEASE` without a dictation, since it holds hundreds of megabytes.
//! One request runs at a time. A model that failed or hung is dropped, so the
//! next request starts from a fresh load.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::time::Instant;

pub const IDLE_RELEASE: Duration = Duration::from_secs(10 * 60);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How a model is loaded on this Mac.
pub trait Runtime: Send + Sync + 'static {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>>;
}

/// A loaded model. Dropping it releases its memory.
pub trait Loaded: Send {
    /// Newer model files landed on disk since this model loaded, such as
    /// the optional one that boosts custom words.
    fn outdated(&self) -> bool {
        false
    }

    /// Transcribes one piece of 16 kHz mono audio, short enough for one pass,
    /// listening for `terms` where the runtime can.
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        terms: &'a [String],
    ) -> BoxFuture<'a, anyhow::Result<String>>;
}

/// What the lock guards: the model, and whether a task waits to release it.
#[derive(Default)]
struct Slot {
    loaded: Option<Box<dyn Loaded>>,
    releaser: bool,
}

pub struct Session {
    runtime: Box<dyn Runtime>,
    slot: tokio::sync::Mutex<Slot>,
    last_used: Mutex<Instant>,
    recording: Box<dyn Fn() -> bool + Send + Sync>,
    idle_release: Duration,
}

impl Session {
    /// `recording` tells whether a dictation is being recorded: the model
    /// is never released under it.
    pub fn new(
        runtime: Box<dyn Runtime>,
        recording: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Arc<Self> {
        Self::with_idle_release(runtime, recording, IDLE_RELEASE)
    }

    /// A session that releases its model after `idle_release` instead.
    pub(super) fn with_idle_release(
        runtime: Box<dyn Runtime>,
        recording: Box<dyn Fn() -> bool + Send + Sync>,
        idle_release: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            slot: tokio::sync::Mutex::new(Slot::default()),
            last_used: Mutex::new(Instant::now()),
            recording,
            idle_release,
        })
    }

    fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
    }

    fn release_at(&self) -> Instant {
        *self
            .last_used
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            + self.idle_release
    }

    /// Loads the model unless it is loaded, and puts its release back.
    pub async fn warm(self: &Arc<Self>) -> anyhow::Result<()> {
        let mut slot = self.slot.lock().await;
        self.ensure_loaded(&mut slot).await
    }

    async fn ensure_loaded(self: &Arc<Self>, slot: &mut Slot) -> anyhow::Result<()> {
        self.touch();
        match &slot.loaded {
            Some(loaded) if loaded.outdated() => {
                log::info!("Newer local model files are on disk: loading them");
                slot.loaded = None;
            }
            Some(_) => return Ok(()),
            None => {}
        }
        let started = std::time::Instant::now();
        slot.loaded = Some(self.runtime.load().await?);
        log::info!("Local model loaded in {:?}", started.elapsed());
        self.touch();
        if !slot.releaser {
            slot.releaser = true;
            tokio::spawn(release_when_idle(Arc::downgrade(self)));
        }
        Ok(())
    }

    /// Transcribes one piece, loading the model first if needed. A run that
    /// takes longer than `limit` fails, and its model is dropped.
    pub async fn transcribe(
        self: &Arc<Self>,
        samples: &[f32],
        terms: &[String],
        limit: Duration,
    ) -> anyhow::Result<String> {
        let mut slot = self.slot.lock().await;
        self.ensure_loaded(&mut slot).await?;
        let Some(loaded) = slot.loaded.as_mut() else {
            anyhow::bail!("The local model is not loaded");
        };
        let result = match tokio::time::timeout(limit, loaded.transcribe(samples, terms)).await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("Local transcription took over {limit:?}")),
        };
        if result.is_err() {
            slot.loaded = None;
        }
        self.touch();
        result
    }

    /// Releases the model now, after the request running, if any, then runs
    /// `then` before any request can load it again. The next request loads
    /// it again.
    pub async fn release<T>(&self, then: impl Future<Output = T>) -> T {
        let mut slot = self.slot.lock().await;
        if slot.loaded.take().is_some() {
            log::info!("Local model released");
        }
        then.await
    }

    #[cfg(test)]
    pub(super) async fn is_loaded(&self) -> bool {
        self.slot.lock().await.loaded.is_some()
    }
}

async fn release_when_idle(session: Weak<Session>) {
    loop {
        let Some(release_at) = session.upgrade().map(|session| session.release_at()) else {
            return;
        };
        tokio::time::sleep_until(release_at).await;
        let Some(session) = session.upgrade() else {
            return;
        };
        if (session.recording)() {
            session.touch();
            continue;
        }
        // A dictation holds the lock until it is done, and then puts the
        // release back.
        let mut slot = session.slot.lock().await;
        if slot.loaded.is_some() && session.release_at() > Instant::now() {
            continue;
        }
        if slot.loaded.take().is_some() {
            log::info!(
                "Local model released after {:?} without a dictation",
                session.idle_release
            );
        }
        slot.releaser = false;
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counts {
        loads: AtomicUsize,
        released: AtomicUsize,
        running: AtomicUsize,
        peak: AtomicUsize,
        /// Bumped when newer model files land on disk.
        on_disk: AtomicUsize,
    }

    /// Each run takes `run_for`; a run of `fail` audio fails.
    struct FakeRuntime {
        counts: Arc<Counts>,
        run_for: Duration,
    }

    struct FakeModel {
        counts: Arc<Counts>,
        run_for: Duration,
        loaded_from: usize,
    }

    impl Runtime for FakeRuntime {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            self.counts.loads.fetch_add(1, Ordering::SeqCst);
            let model = FakeModel {
                counts: Arc::clone(&self.counts),
                run_for: self.run_for,
                loaded_from: self.counts.on_disk.load(Ordering::SeqCst),
            };
            Box::pin(async move { Ok(Box::new(model) as Box<dyn Loaded>) })
        }
    }

    impl Loaded for FakeModel {
        fn outdated(&self) -> bool {
            self.counts.on_disk.load(Ordering::SeqCst) > self.loaded_from
        }

        fn transcribe<'a>(
            &'a mut self,
            samples: &'a [f32],
            _: &'a [String],
        ) -> BoxFuture<'a, anyhow::Result<String>> {
            Box::pin(async move {
                let now = self.counts.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.counts.peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(self.run_for).await;
                self.counts.running.fetch_sub(1, Ordering::SeqCst);
                if samples.first() == Some(&-1.0) {
                    anyhow::bail!("the model failed");
                }
                Ok(format!("{} samples", samples.len()))
            })
        }
    }

    impl Drop for FakeModel {
        fn drop(&mut self) {
            self.counts.released.fetch_add(1, Ordering::SeqCst);
        }
    }

    const LIMIT: Duration = Duration::from_secs(60);

    fn session(run_for: Duration) -> (Arc<Session>, Arc<Counts>, Arc<AtomicBool>) {
        let counts = Arc::new(Counts::default());
        let recording = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&recording);
        let session = Session::new(
            Box::new(FakeRuntime {
                counts: Arc::clone(&counts),
                run_for,
            }),
            Box::new(move || flag.load(Ordering::SeqCst)),
        );
        (session, counts, recording)
    }

    async fn wait(duration: Duration) {
        tokio::time::sleep(duration).await;
        // Lets the releaser run what became due.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn minutes(minutes: u64) -> Duration {
        Duration::from_secs(minutes * 60)
    }

    #[tokio::test(start_paused = true)]
    async fn the_model_loads_on_warm_and_is_reused_by_the_dictation() {
        let (session, counts, _) = session(Duration::from_millis(100));
        assert!(!session.is_loaded().await);
        session.warm().await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 1);
        assert_eq!(
            session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap(),
            "4 samples"
        );
        session.warm().await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_model_is_released_ten_minutes_after_the_last_dictation() {
        let (session, counts, _) = session(Duration::from_millis(100));
        session.warm().await.unwrap();
        wait(minutes(6)).await;
        session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap();
        wait(minutes(9)).await;
        assert!(session.is_loaded().await, "released too early");
        wait(minutes(2)).await;
        assert!(!session.is_loaded().await, "still loaded");
        assert_eq!(counts.released.load(Ordering::SeqCst), 1);

        // The next dictation loads it again.
        session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 2);
        wait(minutes(11)).await;
        assert_eq!(counts.released.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_model_is_never_released_while_recording() {
        let (session, counts, recording) = session(Duration::from_millis(100));
        recording.store(true, Ordering::SeqCst);
        session.warm().await.unwrap();
        wait(minutes(45)).await;
        assert!(session.is_loaded().await);
        recording.store(false, Ordering::SeqCst);
        wait(minutes(11)).await;
        assert!(!session.is_loaded().await);
        assert_eq!(counts.released.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn two_dictations_run_one_after_the_other() {
        let (session, counts, _) = session(Duration::from_secs(2));
        let (first, second) = tokio::join!(
            session.transcribe(&[0.0; 4], &[], LIMIT),
            session.transcribe(&[0.0; 8], &[], LIMIT)
        );
        assert_eq!(first.unwrap(), "4 samples");
        assert_eq!(second.unwrap(), "8 samples");
        assert_eq!(counts.peak.load(Ordering::SeqCst), 1);
        assert_eq!(counts.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_model_outdated_on_disk_is_loaded_again_by_the_next_dictation() {
        let (session, counts, _) = session(Duration::from_millis(100));
        session.warm().await.unwrap();
        // The custom-word model finished downloading.
        counts.on_disk.fetch_add(1, Ordering::SeqCst);
        session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 2);
        assert_eq!(counts.released.load(Ordering::SeqCst), 1);
        session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 2, "loaded again");
    }

    #[tokio::test(start_paused = true)]
    async fn a_released_model_is_dropped_at_once_and_loaded_again_when_needed() {
        let (session, counts, _) = session(Duration::from_millis(100));
        session.warm().await.unwrap();
        // The models were deleted: the mapped file must not wait ten minutes.
        session.release(async {}).await;
        assert!(!session.is_loaded().await);
        assert_eq!(counts.released.load(Ordering::SeqCst), 1);
        session.release(async {}).await;

        session.transcribe(&[0.0; 4], &[], LIMIT).await.unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 2);
        wait(minutes(11)).await;
        assert_eq!(counts.released.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dictation_during_the_delete_loads_the_model_once_it_is_done() {
        let (session, counts, _) = session(Duration::from_millis(100));
        session.warm().await.unwrap();
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        let deleting = tokio::spawn({
            let (session, counts) = (Arc::clone(&session), Arc::clone(&counts));
            async move {
                session
                    .release(async move {
                        // Windows may refuse to delete a loaded model's file.
                        assert_eq!(counts.released.load(Ordering::SeqCst), 1);
                        finished.await.unwrap();
                    })
                    .await;
            }
        });
        wait(Duration::from_millis(10)).await;
        let dictation = tokio::spawn({
            let session = Arc::clone(&session);
            async move { session.transcribe(&[0.0; 4], &[], LIMIT).await }
        });
        wait(Duration::from_secs(5)).await;
        assert_eq!(counts.loads.load(Ordering::SeqCst), 1, "loaded mid-delete");

        finish.send(()).unwrap();
        deleting.await.unwrap();
        dictation.await.unwrap().unwrap();
        assert_eq!(counts.loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_or_hung_model_is_dropped_and_loaded_again() {
        let (session, counts, _) = session(Duration::from_secs(5));
        assert!(session.transcribe(&[-1.0], &[], LIMIT).await.is_err());
        assert!(!session.is_loaded().await);
        assert_eq!(counts.released.load(Ordering::SeqCst), 1);

        let hung = session
            .transcribe(&[0.0], &[], Duration::from_secs(1))
            .await;
        assert!(hung.unwrap_err().to_string().contains("took over"));
        assert!(!session.is_loaded().await);

        assert_eq!(
            session.transcribe(&[0.0], &[], LIMIT).await.unwrap(),
            "1 samples"
        );
        assert_eq!(counts.loads.load(Ordering::SeqCst), 3);
    }
}
