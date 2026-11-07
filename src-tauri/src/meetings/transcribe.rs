//! Sends the pieces of a meeting to Codex: at most three at a time, earliest
//! first across both tracks, each result handed back as soon as it arrives so
//! a crash loses only the pieces in flight.
//!
//! Failures are retried by kind. Rate limits pause every piece and back off
//! for up to about half an hour, then stop the run with the pieces not yet
//! done left pending; network and server errors retry for about five
//! minutes, then a lost connection stops the run the same way, and the
//! queue goes on once it is back; a piece refused as too large is halved; a
//! lost sign-in stops everything, since no other piece could succeed.
//!
//! With the local model ready, a piece Codex refuses for good (rejected,
//! failed, signed out, out of reach, or rate limited past the budget) is
//! transcribed on this Mac instead, and once Codex is out every piece left
//! goes there.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::merge::Track;
use crate::audio_toolkit::gain::{normalize_quiet_speech, GainProfile};
use crate::transcription_error::{FailureKind, TranscriptionError};
use crate::ProcessingOperation;

/// Requests in flight at once, across both tracks.
pub const MAX_PARALLEL: usize = 3;

pub type TranscribeFuture =
    Pin<Box<dyn Future<Output = Result<String, TranscriptionError>> + Send>>;

pub trait Transcriber: Send + Sync + 'static {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        language: Option<String>,
        operation: ProcessingOperation,
    ) -> TranscribeFuture;
}

/// The Codex transcription endpoint, with the session Codex keeps on disk.
pub struct CodexTranscriber {
    pub url: String,
    /// The Codex default when `None`.
    pub auth_path: Option<PathBuf>,
}

impl Transcriber for CodexTranscriber {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        language: Option<String>,
        operation: ProcessingOperation,
    ) -> TranscribeFuture {
        let url = self.url.clone();
        let auth_path = self.auth_path.clone();
        Box::pin(async move {
            crate::codex_transcribe::transcribe_at(
                &url,
                auth_path.as_deref(),
                &samples,
                language.as_deref(),
                MAX_PARALLEL,
                &operation,
            )
            .await
            .map_err(|error| {
                TranscriptionError::new(TranscriptionError::kind_of(&error), error.to_string())
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// The first wait after a rate limit; each later one doubles, up to the cap.
    pub rate_limit_first: Duration,
    pub rate_limit_cap: Duration,
    /// A piece gives up once its rate-limit waits would pass this.
    pub rate_limit_budget: Duration,
    /// Waits before each retry of a network, timeout or server failure.
    pub transient: Vec<Duration>,
}

impl Default for RetryPolicy {
    /// About five and a half minutes of retries: a Wi-Fi drop or a switch
    /// of network recovers within the run.
    fn default() -> Self {
        Self {
            rate_limit_first: Duration::from_secs(15),
            rate_limit_cap: Duration::from_secs(4 * 60),
            rate_limit_budget: Duration::from_secs(30 * 60),
            transient: vec![
                Duration::from_secs(5),
                Duration::from_secs(20),
                Duration::from_secs(60),
                Duration::from_secs(120),
                Duration::from_secs(120),
            ],
        }
    }
}

/// A failure that, once its retries are spent, says Codex is out of reach
/// rather than refusing this piece. A timeout is not one: Codex was reached,
/// and the connection check would find it online and rerun the same request
/// for ever.
pub fn unreachable(kind: FailureKind) -> bool {
    kind == FailureKind::Network
}

/// This Mac's model, with its own engine so a meeting never waits behind a
/// dictation. The language is Parakeet's to detect.
pub struct LocalTranscriber(pub Arc<crate::engine::local::LocalEngine>);

impl Transcriber for LocalTranscriber {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        _language: Option<String>,
        operation: ProcessingOperation,
    ) -> TranscribeFuture {
        let engine = Arc::clone(&self.0);
        Box::pin(async move {
            engine
                .transcribe(&samples, &[], None, &operation)
                .await
                .map_err(|error| TranscriptionError::new(FailureKind::Other, format!("{error:#}")))
        })
    }
}

impl RetryPolicy {
    /// The wait after `earlier` rate limits in a row.
    pub fn rate_limit_delay(&self, earlier: u32) -> Duration {
        self.rate_limit_first
            .saturating_mul(1 << earlier.min(16))
            .min(self.rate_limit_cap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    pub track: Track,
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done(String),
    /// Codex ended with `codex`, and this Mac transcribed the piece.
    DoneLocally {
        text: String,
        codex: FailureKind,
    },
    Failed(FailureKind),
}

/// The audio of the pieces and where a refused piece may be halved.
pub trait ChunkSource: Send + Sync + 'static {
    fn samples(&self, track: Track, start_ms: u64, end_ms: u64) -> anyhow::Result<Vec<f32>>;
    fn split(&self, track: Track, start_ms: u64, end_ms: u64) -> Option<u64>;
}

pub trait RunObserver: Send + Sync + 'static {
    /// A piece finished, for good or after its last retry.
    fn finished(&self, job: &Job, outcome: &Outcome);
    /// Codex is rate limiting: every piece waits.
    fn rate_limited(&self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// Every piece was tried; some may have failed.
    Finished,
    /// The Codex session is gone; pieces not yet sent stay pending.
    SignedOut,
    /// A piece used up its rate-limit budget: Codex will refuse the others
    /// too, so they stay pending for a later retry.
    RateLimited,
    /// A piece used up its network retries: the others stay pending until
    /// the connection is back.
    Unreachable,
    Cancelled,
}

/// What Codex said while a meeting was processed, which tells the queue
/// whether its usage cap has freed up. Later variants win over earlier ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodexVerdict {
    /// Codex was not asked, or failed some other way: nothing learned.
    Unknown,
    /// Codex answered at least once.
    Answered,
    /// Codex could not be reached once the retries were spent.
    Unreachable,
    /// Codex refused with a rate limit.
    RateLimited,
}

pub struct Runner {
    pub transcriber: Arc<dyn Transcriber>,
    /// Takes the pieces Codex refuses for good; `None` without a ready model.
    pub local: Option<Arc<dyn Transcriber>>,
    pub source: Arc<dyn ChunkSource>,
    pub observer: Arc<dyn RunObserver>,
    pub policy: RetryPolicy,
    pub language: Option<String>,
}

/// The moment until which no request may leave, after a rate limit.
#[derive(Default)]
struct RateGate(Mutex<Option<Instant>>);

impl RateGate {
    fn hold(&self, delay: Duration) {
        let until = Instant::now() + delay;
        let mut gate = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *gate = Some(gate.map_or(until, |current| current.max(until)));
    }

    async fn wait(&self, operation: &ProcessingOperation) {
        loop {
            let until = *self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match until {
                Some(until) if until > Instant::now() => {
                    tokio::select! {
                        _ = operation.cancelled() => return,
                        _ = tokio::time::sleep_until(until) => {}
                    }
                }
                _ => return,
            }
        }
    }
}

/// Shared by every piece of one run.
struct Shared {
    runner: Runner,
    operation: ProcessingOperation,
    signed_out: AtomicBool,
    rate_limited_out: AtomicBool,
    unreachable_out: AtomicBool,
    /// A piece ended without text, or was never sent.
    incomplete: AtomicBool,
    gate: RateGate,
}

enum Attempt {
    Outcome(Outcome),
    /// Cancelled or stopped: nothing to record.
    Stopped,
}

impl Shared {
    /// Why Codex will refuse every piece left in this run.
    fn codex_out(&self) -> Option<FailureKind> {
        if self.signed_out.load(Ordering::SeqCst) {
            Some(FailureKind::SignedOut)
        } else if self.rate_limited_out.load(Ordering::SeqCst) {
            Some(FailureKind::RateLimited)
        } else if self.unreachable_out.load(Ordering::SeqCst) {
            Some(FailureKind::Network)
        } else {
            None
        }
    }

    fn stopped(&self) -> bool {
        self.operation.is_cancelled() || (self.runner.local.is_none() && self.codex_out().is_some())
    }

    /// A piece Codex refused for good with `codex`: transcribed on this Mac
    /// when it can, failed with Codex's reason otherwise.
    async fn refused(&self, samples: Vec<f32>, codex: FailureKind) -> Attempt {
        let Some(local) = &self.runner.local else {
            return Attempt::Outcome(Outcome::Failed(codex));
        };
        match local
            .transcribe(samples, None, self.operation.clone())
            .await
        {
            Ok(text) => Attempt::Outcome(Outcome::DoneLocally {
                text: text.trim().to_string(),
                codex,
            }),
            Err(_) if self.operation.is_cancelled() => Attempt::Stopped,
            Err(error) => {
                log::warn!(
                    "Local transcription of a meeting piece failed: {}",
                    error.detail
                );
                Attempt::Outcome(Outcome::Failed(codex))
            }
        }
    }

    async fn sleep(&self, delay: Duration) {
        tokio::select! {
            _ = self.operation.cancelled() => {}
            _ = tokio::time::sleep(delay) => {}
        }
    }

    fn transcribe_range(
        self: Arc<Self>,
        track: Track,
        start_ms: u64,
        end_ms: u64,
    ) -> Pin<Box<dyn Future<Output = Attempt> + Send>> {
        Box::pin(async move {
            let samples = match self.runner.source.samples(track, start_ms, end_ms) {
                Ok(samples) => samples,
                Err(error) => {
                    log::error!("Cannot read meeting audio: {error}");
                    return Attempt::Outcome(Outcome::Failed(FailureKind::Other));
                }
            };
            let samples =
                normalize_quiet_speech(&samples, GainProfile::new(false)).unwrap_or(samples);
            let policy = &self.runner.policy;
            let mut transient = 0;
            let mut rate_limits = 0u32;
            let mut waited = Duration::ZERO;
            loop {
                if self.codex_out().is_none() {
                    self.gate.wait(&self.operation).await;
                }
                if self.stopped() {
                    return Attempt::Stopped;
                }
                if let Some(codex) = self.codex_out() {
                    return self.refused(samples, codex).await;
                }
                let result = self
                    .runner
                    .transcriber
                    .transcribe(
                        samples.clone(),
                        self.runner.language.clone(),
                        self.operation.clone(),
                    )
                    .await;
                let error = match result {
                    Ok(text) => return Attempt::Outcome(Outcome::Done(text.trim().to_string())),
                    Err(_) if self.operation.is_cancelled() => return Attempt::Stopped,
                    Err(error) => error,
                };
                log::warn!(
                    "Meeting piece {track:?} {start_ms}-{end_ms} ms failed: {:?}",
                    error.kind
                );
                match error.kind {
                    FailureKind::RateLimited => {
                        let delay = policy.rate_limit_delay(rate_limits);
                        if waited + delay > policy.rate_limit_budget {
                            self.rate_limited_out.store(true, Ordering::SeqCst);
                            return self.refused(samples, error.kind).await;
                        }
                        waited += delay;
                        rate_limits += 1;
                        self.gate.hold(delay);
                        self.runner.observer.rate_limited();
                    }
                    FailureKind::Network
                    | FailureKind::Timeout
                    | FailureKind::ServerError
                    | FailureKind::ConnectionDropped => {
                        let Some(delay) = policy.transient.get(transient).copied() else {
                            if error.kind == FailureKind::ServerError {
                                return Attempt::Outcome(Outcome::Failed(error.kind));
                            }
                            // After a timeout or a dropped connection, the
                            // other pieces still try Codex; a connection
                            // that never opened stops them.
                            if unreachable(error.kind) {
                                self.unreachable_out.store(true, Ordering::SeqCst);
                            }
                            return self.refused(samples, error.kind).await;
                        };
                        transient += 1;
                        self.sleep(delay).await;
                    }
                    FailureKind::TooLarge => {
                        let Some(cut) = self.runner.source.split(track, start_ms, end_ms) else {
                            return Attempt::Outcome(Outcome::Failed(error.kind));
                        };
                        let first = Arc::clone(&self)
                            .transcribe_range(track, start_ms, cut)
                            .await;
                        let Attempt::Outcome(
                            first @ (Outcome::Done(_) | Outcome::DoneLocally { .. }),
                        ) = first
                        else {
                            return first;
                        };
                        let second = Arc::clone(&self).transcribe_range(track, cut, end_ms).await;
                        let Attempt::Outcome(second) = second else {
                            return second;
                        };
                        return Attempt::Outcome(joined(first, second));
                    }
                    FailureKind::SessionExpired | FailureKind::SignedOut => {
                        self.signed_out.store(true, Ordering::SeqCst);
                        return self.refused(samples, error.kind).await;
                    }
                    FailureKind::Rejected
                    | FailureKind::LocalNotReady
                    | FailureKind::LocalFailed
                    | FailureKind::LanguageNeedsCloud
                    | FailureKind::Other => {
                        return self.refused(samples, error.kind).await;
                    }
                }
            }
        })
    }
}

/// The two halves of a piece as one piece: done on this Mac when either half
/// was, failed when either failed, since a done piece is never sent again.
fn joined(first: Outcome, second: Outcome) -> Outcome {
    let mut local = None;
    let mut texts = Vec::with_capacity(2);
    for half in [first, second] {
        match half {
            Outcome::Done(text) => texts.push(text),
            Outcome::DoneLocally { text, codex } => {
                local.get_or_insert(codex);
                texts.push(text);
            }
            failed @ Outcome::Failed(_) => return failed,
        }
    }
    texts.retain(|text| !text.is_empty());
    let text = texts.join(" ");
    match local {
        Some(codex) => Outcome::DoneLocally { text, codex },
        None => Outcome::Done(text),
    }
}

impl Runner {
    /// Transcribes `jobs` in the order given and reports each through the
    /// observer.
    pub async fn run(self, jobs: Vec<Job>, operation: ProcessingOperation) -> RunEnd {
        let shared = Arc::new(Shared {
            runner: self,
            operation,
            signed_out: AtomicBool::new(false),
            rate_limited_out: AtomicBool::new(false),
            unreachable_out: AtomicBool::new(false),
            incomplete: AtomicBool::new(false),
            gate: RateGate::default(),
        });
        let slots = Arc::new(Semaphore::new(MAX_PARALLEL));
        let mut running = JoinSet::new();
        for job in jobs {
            let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
                break;
            };
            if shared.stopped() {
                shared.incomplete.store(true, Ordering::SeqCst);
                break;
            }
            let shared = Arc::clone(&shared);
            running.spawn(async move {
                let attempt = Arc::clone(&shared)
                    .transcribe_range(job.track, job.start_ms, job.end_ms)
                    .await;
                match attempt {
                    Attempt::Outcome(outcome) => {
                        if matches!(outcome, Outcome::Failed(_)) {
                            shared.incomplete.store(true, Ordering::SeqCst);
                        }
                        shared.runner.observer.finished(&job, &outcome);
                    }
                    Attempt::Stopped => shared.incomplete.store(true, Ordering::SeqCst),
                }
                drop(slot);
            });
        }
        while running.join_next().await.is_some() {}
        // Codex going out ends the run that way only when it left pieces
        // without text; this Mac may have transcribed them all.
        let incomplete = shared.incomplete.load(Ordering::SeqCst);
        if shared.operation.is_cancelled() {
            RunEnd::Cancelled
        } else if incomplete && shared.signed_out.load(Ordering::SeqCst) {
            RunEnd::SignedOut
        } else if incomplete && shared.rate_limited_out.load(Ordering::SeqCst) {
            RunEnd::RateLimited
        } else if incomplete && shared.unreachable_out.load(Ordering::SeqCst) {
            RunEnd::Unreachable
        } else {
            RunEnd::Finished
        }
    }
}

#[cfg(test)]
mod tests;
