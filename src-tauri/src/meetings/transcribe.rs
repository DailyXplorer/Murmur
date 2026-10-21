//! Sends the pieces of a meeting to Codex: at most three at a time, earliest
//! first across both tracks, each result handed back as soon as it arrives so
//! a crash loses only the pieces in flight.
//!
//! Failures are retried by kind. Rate limits pause every piece and back off
//! for up to about half an hour, then stop the run with the pieces not yet
//! done left pending; network and server errors retry a few times;
//! a piece refused as too large is halved; a lost sign-in stops everything,
//! since no other piece could succeed.
//!
//! With the local model ready, a piece Codex refuses for good (rejected,
//! failed, signed out, or rate limited past the budget) is transcribed on
//! this Mac instead, and once Codex is out every piece left goes there.

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
    fn default() -> Self {
        Self {
            rate_limit_first: Duration::from_secs(15),
            rate_limit_cap: Duration::from_secs(4 * 60),
            rate_limit_budget: Duration::from_secs(30 * 60),
            transient: vec![
                Duration::from_secs(5),
                Duration::from_secs(20),
                Duration::from_secs(60),
            ],
        }
    }
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
                .transcribe(&samples, &operation)
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
                    FailureKind::Network | FailureKind::Timeout | FailureKind::ServerError => {
                        let Some(delay) = policy.transient.get(transient).copied() else {
                            return Attempt::Outcome(Outcome::Failed(error.kind));
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
                    FailureKind::Rejected | FailureKind::Other => {
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
        } else {
            RunEnd::Finished
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::chunking::split_chunk;
    use crate::OperationId;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Condvar};

    /// One request as the fake server saw it.
    #[derive(Debug, Clone)]
    struct Request {
        token: String,
        body_bytes: usize,
    }

    type Reply = Box<dyn Fn(usize, &Request) -> (u16, String) + Send + Sync>;

    /// A local stand-in for the transcription endpoint. `reply` gets the
    /// request number (from 0) and decides the answer, and may block to make
    /// requests overlap.
    struct FakeServer {
        url: String,
        requests: Arc<Mutex<Vec<Request>>>,
        peak: Arc<AtomicUsize>,
    }

    fn serve(reply: Reply) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let reply = Arc::new(reply);
        let (seen, high) = (Arc::clone(&requests), Arc::clone(&peak));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (seen, high, in_flight, reply) = (
                    Arc::clone(&seen),
                    Arc::clone(&high),
                    Arc::clone(&in_flight),
                    Arc::clone(&reply),
                );
                std::thread::spawn(move || {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    high.fetch_max(now, Ordering::SeqCst);
                    handle(stream, &seen, &in_flight, reply.as_ref());
                });
            }
        });
        FakeServer {
            url,
            requests,
            peak,
        }
    }

    fn handle(
        stream: TcpStream,
        seen: &Mutex<Vec<Request>>,
        in_flight: &AtomicUsize,
        reply: &(dyn Fn(usize, &Request) -> (u16, String) + Send + Sync),
    ) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let (mut length, mut token, mut chunked) = (0, String::new(), false);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                in_flight.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                chunked = true;
            }
            if let Some(value) = line
                .strip_prefix("authorization: Bearer ")
                .or_else(|| line.strip_prefix("Authorization: Bearer "))
            {
                token = value.to_string();
            }
        }
        if chunked {
            length = 0;
            loop {
                let mut size = String::new();
                if reader.read_line(&mut size).unwrap_or(0) == 0 {
                    break;
                }
                let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                let mut data = vec![0; size + 2];
                let _ = reader.read_exact(&mut data);
                length += size;
                if size == 0 {
                    break;
                }
            }
        } else {
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
        }
        let request = Request {
            token,
            body_bytes: length,
        };
        let number = {
            let mut seen = seen.lock().unwrap();
            seen.push(request.clone());
            seen.len() - 1
        };
        let (status, body) = reply(number, &request);
        // Before the answer leaves: the slot it frees may send the next
        // request at once.
        in_flight.fetch_sub(1, Ordering::SeqCst);
        let mut stream = stream;
        let _ = write!(
            stream,
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    }

    /// Holds the first `count` requests until all of them have arrived, so
    /// they overlap however slowly the machine runs.
    struct Rendezvous {
        count: usize,
        arrived: Mutex<usize>,
        all_here: Condvar,
    }

    impl Rendezvous {
        fn new(count: usize) -> Self {
            Self {
                count,
                arrived: Mutex::new(0),
                all_here: Condvar::new(),
            }
        }

        /// Waits for the others; true for the last to arrive.
        fn arrive(&self) -> bool {
            let mut arrived = self.arrived.lock().unwrap();
            *arrived += 1;
            if *arrived == self.count {
                self.all_here.notify_all();
                return true;
            }
            // Only a runner that never sends them all reaches the timeout.
            let (arrived, wait) = self
                .all_here
                .wait_timeout_while(arrived, Duration::from_secs(30), |n| *n < self.count)
                .unwrap();
            assert!(!wait.timed_out(), "{} of {} requests", *arrived, self.count);
            false
        }
    }

    fn ok(text: &str) -> (u16, String) {
        (200, serde_json::json!({ "text": text }).to_string())
    }

    /// Written whole, then moved in place, as Codex does, so a concurrent
    /// read never sees half a file.
    fn auth_file(dir: &Path, token: &str) {
        let staging = dir.join(format!("auth.json.{token}"));
        std::fs::write(
            &staging,
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": token,
                    "refresh_token": "refresh",
                    "account_id": "acct_test"
                },
                "last_refresh": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
        )
        .unwrap();
        std::fs::rename(staging, dir.join("auth.json")).unwrap();
    }

    /// Silent 16 kHz audio of the asked length; refused pieces are halved
    /// like the real planner does.
    struct Silence;

    impl ChunkSource for Silence {
        fn samples(&self, _: Track, start_ms: u64, end_ms: u64) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; ((end_ms - start_ms) * 16) as usize])
        }
        fn split(&self, _: Track, start_ms: u64, end_ms: u64) -> Option<u64> {
            split_chunk(&[], start_ms, end_ms).map(|(_, cut, _)| cut)
        }
    }

    #[derive(Default)]
    struct Recorder {
        finished: Mutex<Vec<(Job, Outcome)>>,
        rate_limited: AtomicUsize,
    }

    impl RunObserver for Recorder {
        fn finished(&self, job: &Job, outcome: &Outcome) {
            self.finished.lock().unwrap().push((*job, outcome.clone()));
        }
        fn rate_limited(&self) {
            self.rate_limited.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn quick_policy() -> RetryPolicy {
        RetryPolicy {
            rate_limit_first: Duration::from_millis(20),
            rate_limit_cap: Duration::from_millis(40),
            rate_limit_budget: Duration::from_millis(200),
            transient: vec![Duration::from_millis(5); 3],
        }
    }

    fn jobs(count: u32, length_ms: u64) -> Vec<Job> {
        (0..count)
            .map(|idx| Job {
                track: if idx % 2 == 0 {
                    Track::Mic
                } else {
                    Track::System
                },
                idx,
                start_ms: u64::from(idx) * length_ms,
                end_ms: u64::from(idx + 1) * length_ms,
            })
            .collect()
    }

    struct Run {
        end: RunEnd,
        recorder: Arc<Recorder>,
        _dir: tempfile::TempDir,
    }

    fn run(server: &FakeServer, jobs: Vec<Job>, operation: ProcessingOperation) -> Run {
        run_with_local(server, jobs, operation, None)
    }

    fn run_with_local(
        server: &FakeServer,
        jobs: Vec<Job>,
        operation: ProcessingOperation,
        local: Option<Arc<dyn Transcriber>>,
    ) -> Run {
        let dir = tempfile::tempdir().unwrap();
        auth_file(dir.path(), "token-0");
        let recorder = Arc::new(Recorder::default());
        let runner = Runner {
            transcriber: Arc::new(CodexTranscriber {
                url: server.url.clone(),
                auth_path: Some(dir.path().join("auth.json")),
            }),
            local,
            source: Arc::new(Silence),
            observer: Arc::clone(&recorder) as Arc<dyn RunObserver>,
            policy: quick_policy(),
            language: Some("fr".into()),
        };
        let end = tauri::async_runtime::block_on(runner.run(jobs, operation));
        Run {
            end,
            recorder,
            _dir: dir,
        }
    }

    fn outcomes(run: &Run) -> Vec<(u32, Outcome)> {
        let mut outcomes: Vec<_> = run
            .recorder
            .finished
            .lock()
            .unwrap()
            .iter()
            .map(|(job, outcome)| (job.idx, outcome.clone()))
            .collect();
        outcomes.sort_by_key(|(idx, _)| *idx);
        outcomes
    }

    fn operation() -> ProcessingOperation {
        ProcessingOperation::new(OperationId(42))
    }

    #[test]
    fn at_most_three_pieces_are_in_flight_and_all_are_transcribed() {
        let together = Rendezvous::new(MAX_PARALLEL);
        let server = serve(Box::new(move |number, _| {
            if number < MAX_PARALLEL {
                together.arrive();
            }
            ok(&format!("texte {number}"))
        }));
        let run = run(&server, jobs(8, 1_000), operation());
        assert_eq!(run.end, RunEnd::Finished);
        assert_eq!(server.peak.load(Ordering::SeqCst), MAX_PARALLEL);
        let outcomes = outcomes(&run);
        assert_eq!(outcomes.len(), 8);
        assert!(outcomes.iter().all(
            |(_, outcome)| matches!(outcome, Outcome::Done(text) if text.starts_with("texte"))
        ));
    }

    #[test]
    fn rate_limits_back_off_then_succeed() {
        let server = serve(Box::new(|number, _| {
            if number < 2 {
                (429, r#"{"detail":"slow down"}"#.into())
            } else {
                ok("Bonjour.")
            }
        }));
        let run = run(&server, jobs(1, 1_000), operation());
        assert_eq!(run.end, RunEnd::Finished);
        assert_eq!(outcomes(&run), [(0, Outcome::Done("Bonjour.".into()))]);
        assert_eq!(run.recorder.rate_limited.load(Ordering::SeqCst), 2);
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_lasting_rate_limit_fails_the_piece_after_its_budget() {
        let server = serve(Box::new(|_, _| (429, r#"{"detail":"slow down"}"#.into())));
        let run = run(&server, jobs(1, 1_000), operation());
        assert_eq!(run.end, RunEnd::RateLimited);
        assert_eq!(
            outcomes(&run),
            [(0, Outcome::Failed(FailureKind::RateLimited))]
        );
        // Waits of 20, 40, 40, 40, 40 ms fit the 200 ms budget; the sixth would not.
        assert_eq!(server.requests.lock().unwrap().len(), 6);
    }

    #[test]
    fn a_lasting_rate_limit_stops_the_run_and_leaves_later_pieces_pending() {
        let server = serve(Box::new(|_, _| (429, r#"{"detail":"slow down"}"#.into())));
        let run = run(&server, jobs(12, 1_000), operation());
        assert_eq!(run.end, RunEnd::RateLimited);
        // Only the pieces already in flight spend the budget; a fresh budget
        // for each later piece would keep the queue blocked for hours.
        let finished = outcomes(&run);
        assert!(finished.len() <= MAX_PARALLEL, "{finished:?}");
        assert!(finished
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Failed(FailureKind::RateLimited)));
        assert!(server.requests.lock().unwrap().len() <= 6 * MAX_PARALLEL);
    }

    /// This Mac's model: answers with the piece's length, or fails.
    struct FakeLocal {
        fails: bool,
        calls: AtomicUsize,
    }

    impl FakeLocal {
        fn answering() -> Arc<Self> {
            Arc::new(Self {
                fails: false,
                calls: AtomicUsize::new(0),
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                fails: true,
                calls: AtomicUsize::new(0),
            })
        }
    }

    impl Transcriber for FakeLocal {
        fn transcribe(
            &self,
            samples: Vec<f32>,
            _: Option<String>,
            _: ProcessingOperation,
        ) -> TranscribeFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let fails = self.fails;
            Box::pin(async move {
                if fails {
                    Err(TranscriptionError::new(FailureKind::Other, "local failed"))
                } else {
                    Ok(format!(" local {} ", samples.len()))
                }
            })
        }
    }

    #[test]
    fn past_the_rate_limit_budget_every_piece_is_transcribed_on_this_mac() {
        let server = serve(Box::new(|_, _| (429, r#"{"detail":"slow down"}"#.into())));
        let local = FakeLocal::answering();
        let run = run_with_local(
            &server,
            jobs(12, 1_000),
            operation(),
            Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        );
        assert_eq!(run.end, RunEnd::Finished);
        let outcomes = outcomes(&run);
        assert_eq!(outcomes.len(), 12);
        for (_, outcome) in &outcomes {
            assert_eq!(
                *outcome,
                Outcome::DoneLocally {
                    text: "local 16000".into(),
                    codex: FailureKind::RateLimited
                }
            );
        }
        assert_eq!(local.calls.load(Ordering::SeqCst), 12);
        // Once Codex is out, the pieces left are not sent to it.
        assert!(server.requests.lock().unwrap().len() <= 6 * MAX_PARALLEL);
    }

    #[test]
    fn a_refused_piece_goes_to_this_mac_and_a_network_failure_does_not() {
        let server = serve(Box::new(|_, request| {
            // The first piece is twice as long as the second.
            if request.body_bytes > 40_000 {
                (400, "bad request".into())
            } else {
                (502, "bad gateway".into())
            }
        }));
        let jobs = vec![
            Job {
                track: Track::Mic,
                idx: 0,
                start_ms: 0,
                end_ms: 2_000,
            },
            Job {
                track: Track::System,
                idx: 1,
                start_ms: 2_000,
                end_ms: 3_000,
            },
        ];
        let local = FakeLocal::answering();
        let run = run_with_local(
            &server,
            jobs,
            operation(),
            Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        );
        assert_eq!(run.end, RunEnd::Finished);
        assert_eq!(
            outcomes(&run),
            [
                (
                    0,
                    Outcome::DoneLocally {
                        text: "local 32000".into(),
                        codex: FailureKind::Rejected
                    }
                ),
                (1, Outcome::Failed(FailureKind::ServerError)),
            ]
        );
        assert_eq!(local.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_lost_session_with_a_ready_model_still_finishes_the_meeting() {
        let server = serve(Box::new(|_, _| (401, r#"{"detail":"expired"}"#.into())));
        let run = run_with_local(
            &server,
            jobs(5, 1_000),
            operation(),
            Some(FakeLocal::answering() as Arc<dyn Transcriber>),
        );
        assert_eq!(run.end, RunEnd::Finished);
        let outcomes = outcomes(&run);
        assert_eq!(outcomes.len(), 5);
        assert!(outcomes.iter().all(|(_, outcome)| matches!(
            outcome,
            Outcome::DoneLocally { codex, .. }
                if matches!(codex, FailureKind::SessionExpired | FailureKind::SignedOut)
        )));
    }

    #[test]
    fn a_failing_local_model_leaves_the_codex_failure() {
        let server = serve(Box::new(|_, _| (429, r#"{"detail":"slow down"}"#.into())));
        let run = run_with_local(
            &server,
            jobs(2, 1_000),
            operation(),
            Some(FakeLocal::failing() as Arc<dyn Transcriber>),
        );
        assert_eq!(run.end, RunEnd::RateLimited);
        assert!(outcomes(&run)
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Failed(FailureKind::RateLimited)));
    }

    #[test]
    fn server_errors_are_retried_then_the_piece_fails_and_the_others_go_on() {
        let server = serve(Box::new(|_, request| {
            // The first piece is twice as long as the second.
            if request.body_bytes > 40_000 {
                (502, "bad gateway".into())
            } else {
                ok("Suite.")
            }
        }));
        let pieces = vec![
            Job {
                track: Track::Mic,
                idx: 0,
                start_ms: 0,
                end_ms: 2_000,
            },
            Job {
                track: Track::Mic,
                idx: 1,
                start_ms: 2_000,
                end_ms: 3_000,
            },
        ];
        let run = run(&server, pieces, operation());
        assert_eq!(
            outcomes(&run),
            [
                (0, Outcome::Failed(FailureKind::ServerError)),
                (1, Outcome::Done("Suite.".into())),
            ]
        );
        // Each of the 4 meeting attempts includes Codex's own 2 retries.
        let failing = server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.body_bytes > 40_000)
            .count();
        assert_eq!(failing, 4 * 3);
    }

    #[test]
    fn a_piece_refused_as_too_large_is_halved_and_rejoined() {
        let server = serve(Box::new(|number, request| {
            // 150 s of 16 kHz WAV is 4.8 MB; its halves pass.
            if request.body_bytes > 3_000_000 {
                (413, "too large".into())
            } else {
                ok(&format!("moitié {number}"))
            }
        }));
        let piece = Job {
            track: Track::System,
            idx: 0,
            start_ms: 0,
            end_ms: 150_000,
        };
        let run = run(&server, vec![piece], operation());
        assert_eq!(
            outcomes(&run),
            [(0, Outcome::Done("moitié 1 moitié 2".into()))]
        );
    }

    /// A 150 s piece refused as too large, whose request `refused` (1 for
    /// the first half, 2 for the second) Codex then rejects.
    fn halved_with_one_half_rejected(refused: usize) -> (Run, usize) {
        let server = serve(Box::new(move |number, request| {
            if request.body_bytes > 3_000_000 {
                (413, "too large".into())
            } else if number == refused {
                (400, "bad request".into())
            } else {
                ok(&format!("moitié {number}"))
            }
        }));
        let piece = Job {
            track: Track::System,
            idx: 0,
            start_ms: 0,
            end_ms: 150_000,
        };
        let run = run_with_local(
            &server,
            vec![piece],
            operation(),
            Some(FakeLocal::answering() as Arc<dyn Transcriber>),
        );
        let requests = server.requests.lock().unwrap().len();
        (run, requests)
    }

    #[test]
    fn a_halved_piece_whose_first_half_goes_to_this_mac_keeps_the_second_half() {
        let (run, requests) = halved_with_one_half_rejected(1);
        assert_eq!(
            outcomes(&run),
            [(
                0,
                Outcome::DoneLocally {
                    text: "local 1439680 moitié 2".into(),
                    codex: FailureKind::Rejected
                }
            )]
        );
        assert_eq!(requests, 3);
    }

    #[test]
    fn a_halved_piece_whose_second_half_goes_to_this_mac_keeps_the_first_half() {
        let (run, _) = halved_with_one_half_rejected(2);
        assert_eq!(
            outcomes(&run),
            [(
                0,
                Outcome::DoneLocally {
                    text: "moitié 1 local 960320".into(),
                    codex: FailureKind::Rejected
                }
            )]
        );
    }

    #[test]
    fn a_lost_session_stops_every_piece_not_yet_sent() {
        let dir_token = Arc::new(Mutex::new(None::<PathBuf>));
        let auth = Arc::clone(&dir_token);
        let server = serve(Box::new(move |number, _| {
            // Another Codex client refreshed the token: the one-time 401
            // recovery picks it up without the network, and fails again.
            if let Some(dir) = auth.lock().unwrap().as_ref() {
                auth_file(dir, &format!("token-{}", number + 1));
            }
            (401, r#"{"detail":"expired"}"#.into())
        }));
        let dir = tempfile::tempdir().unwrap();
        auth_file(dir.path(), "token-0");
        *dir_token.lock().unwrap() = Some(dir.path().to_path_buf());
        let recorder = Arc::new(Recorder::default());
        let runner = Runner {
            transcriber: Arc::new(CodexTranscriber {
                url: server.url.clone(),
                auth_path: Some(dir.path().join("auth.json")),
            }),
            local: None,
            source: Arc::new(Silence),
            observer: Arc::clone(&recorder) as Arc<dyn RunObserver>,
            policy: quick_policy(),
            language: None,
        };
        let end = tauri::async_runtime::block_on(runner.run(jobs(9, 1_000), operation()));

        assert_eq!(end, RunEnd::SignedOut);
        let finished = recorder.finished.lock().unwrap();
        assert!(finished.len() <= MAX_PARALLEL, "{finished:?}");
        assert!(finished
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Failed(FailureKind::SessionExpired)));
        assert!(server.requests.lock().unwrap().len() <= 2 * MAX_PARALLEL);
    }

    #[test]
    fn a_refreshed_session_is_used_after_one_unauthorized_answer() {
        let dir = tempfile::tempdir().unwrap();
        let auth_dir = dir.path().to_path_buf();
        let server = serve(Box::new(move |_, request| {
            if request.token == "token-0" {
                auth_file(&auth_dir, "token-1");
                (401, r#"{"detail":"expired"}"#.into())
            } else {
                ok("Rebonjour.")
            }
        }));
        auth_file(dir.path(), "token-0");
        let recorder = Arc::new(Recorder::default());
        let runner = Runner {
            transcriber: Arc::new(CodexTranscriber {
                url: server.url.clone(),
                auth_path: Some(dir.path().join("auth.json")),
            }),
            local: None,
            source: Arc::new(Silence),
            observer: Arc::clone(&recorder) as Arc<dyn RunObserver>,
            policy: quick_policy(),
            language: None,
        };
        let end = tauri::async_runtime::block_on(runner.run(jobs(1, 1_000), operation()));
        assert_eq!(end, RunEnd::Finished);
        assert_eq!(
            recorder.finished.lock().unwrap()[0].1,
            Outcome::Done("Rebonjour.".into())
        );
        let tokens: Vec<String> = server
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.token.clone())
            .collect();
        assert_eq!(tokens, ["token-0", "token-1"]);
    }

    #[test]
    fn cancelling_stops_new_requests() {
        let operation = operation();
        let cancel = operation.clone();
        let together = Rendezvous::new(MAX_PARALLEL);
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let server = serve(Box::new(move |number, _| {
            if number >= MAX_PARALLEL {
                // Sent after the cancel: answered so the run still ends.
                return ok("trop tard");
            }
            // Cancelled once every slot is taken; no answer until the run
            // is over, so no slot frees before the cancel.
            if together.arrive() {
                cancel.cancel();
            }
            let _ = released.lock().unwrap().recv();
            ok("x")
        }));
        let run = run(&server, jobs(9, 1_000), operation);
        drop(release);
        assert_eq!(run.end, RunEnd::Cancelled);
        assert_eq!(server.requests.lock().unwrap().len(), MAX_PARALLEL);
        assert!(outcomes(&run).is_empty());
    }
}
