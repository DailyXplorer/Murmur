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
    assert!(outcomes
        .iter()
        .all(|(_, outcome)| matches!(outcome, Outcome::Done(text) if text.starts_with("texte"))));
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

/// Codex behind a connection that is down until `online` is set.
#[derive(Default)]
struct Connection {
    online: AtomicBool,
    calls: AtomicUsize,
}

impl Transcriber for Connection {
    fn transcribe(
        &self,
        _: Vec<f32>,
        _: Option<String>,
        _: ProcessingOperation,
    ) -> TranscribeFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let online = self.online.load(Ordering::SeqCst);
        Box::pin(async move {
            if online {
                Ok("En ligne.".to_string())
            } else {
                Err(TranscriptionError::new(FailureKind::Network, "offline"))
            }
        })
    }
}

fn run_through<C: Transcriber + 'static>(
    codex: &Arc<C>,
    jobs: Vec<Job>,
    local: Option<Arc<dyn Transcriber>>,
) -> (RunEnd, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let runner = Runner {
        transcriber: Arc::clone(codex) as Arc<dyn Transcriber>,
        local,
        source: Arc::new(Silence),
        observer: Arc::clone(&recorder) as Arc<dyn RunObserver>,
        policy: quick_policy(),
        language: None,
    };
    let end = tauri::async_runtime::block_on(runner.run(jobs, operation()));
    (end, recorder)
}

#[test]
fn past_its_network_retries_every_piece_is_transcribed_on_this_mac() {
    let codex = Arc::new(Connection::default());
    let local = FakeLocal::answering();
    let (end, recorder) = run_through(
        &codex,
        jobs(9, 1_000),
        Some(Arc::clone(&local) as Arc<dyn Transcriber>),
    );
    assert_eq!(end, RunEnd::Finished);
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), 9);
    assert!(finished.iter().all(|(_, outcome)| *outcome
        == Outcome::DoneLocally {
            text: "local 16000".into(),
            codex: FailureKind::Network
        }));
    // Once Codex is out of reach, the pieces left are not sent to it.
    assert!(codex.calls.load(Ordering::SeqCst) <= 4 * MAX_PARALLEL);
}

#[test]
fn without_a_model_a_lost_connection_leaves_the_pieces_not_sent_pending() {
    let codex = Arc::new(Connection::default());
    let (end, recorder) = run_through(&codex, jobs(9, 1_000), None);
    assert_eq!(end, RunEnd::Unreachable);
    {
        let finished = recorder.finished.lock().unwrap();
        assert!(finished.len() <= MAX_PARALLEL, "{finished:?}");
        assert!(finished
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Failed(FailureKind::Network)));
    }
    assert!(codex.calls.load(Ordering::SeqCst) <= 4 * MAX_PARALLEL);

    // The connection is back: the next run sends every piece again.
    codex.online.store(true, Ordering::SeqCst);
    let (end, recorder) = run_through(&codex, jobs(9, 1_000), None);
    assert_eq!(end, RunEnd::Finished);
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), 9);
    assert!(finished
        .iter()
        .all(|(_, outcome)| *outcome == Outcome::Done("En ligne.".into())));
}

/// Codex that connects but answers every piece too late, as on a slow
/// uplink.
struct TooSlow;

impl Transcriber for TooSlow {
    fn transcribe(
        &self,
        _: Vec<f32>,
        _: Option<String>,
        _: ProcessingOperation,
    ) -> TranscribeFuture {
        Box::pin(async { Err(TranscriptionError::new(FailureKind::Timeout, "too slow")) })
    }
}

#[test]
fn pieces_that_keep_timing_out_fail_instead_of_waiting_for_a_connection() {
    // The connection works: waiting for it to come back would rerun the
    // same requests for ever.
    let (end, recorder) = run_through(&Arc::new(TooSlow), jobs(4, 1_000), None);
    assert_eq!(end, RunEnd::Finished);
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), 4, "every piece is tried");
    assert!(finished
        .iter()
        .all(|(_, outcome)| *outcome == Outcome::Failed(FailureKind::Timeout)));

    // This Mac's model still takes them.
    let local = FakeLocal::answering();
    let (end, recorder) = run_through(
        &Arc::new(TooSlow),
        jobs(2, 1_000),
        Some(Arc::clone(&local) as Arc<dyn Transcriber>),
    );
    assert_eq!(end, RunEnd::Finished);
    assert!(recorder
        .finished
        .lock()
        .unwrap()
        .iter()
        .all(|(_, outcome)| *outcome
            == Outcome::DoneLocally {
                text: "local 16000".into(),
                codex: FailureKind::Timeout
            }));
}

/// Codex behind a proxy that cuts every upload once the connection
/// opened, classified as the real client does.
struct CutUploads(String);

impl Transcriber for CutUploads {
    fn transcribe(
        &self,
        _: Vec<f32>,
        _: Option<String>,
        _: ProcessingOperation,
    ) -> TranscribeFuture {
        let url = self.0.clone();
        Box::pin(async move {
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let error = client.post(url).send().await.unwrap_err();
            Err(TranscriptionError::new(
                FailureKind::from_transport(&error),
                error.to_string(),
            ))
        })
    }
}

/// Round 51: the pieces go to a ready model, as after a timeout, and
/// not into failed parts.
#[test]
fn pieces_cut_after_the_connection_opened_go_to_a_ready_model() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    let (end, recorder) = run_through(
        &Arc::new(CutUploads(url)),
        jobs(2, 1_000),
        Some(FakeLocal::answering() as Arc<dyn Transcriber>),
    );
    assert_eq!(end, RunEnd::Finished);
    let finished = recorder.finished.lock().unwrap();
    assert_eq!(finished.len(), 2);
    assert!(
        finished.iter().all(|(_, outcome)| matches!(
            outcome,
            Outcome::DoneLocally { text, .. } if text == "local 16000"
        )),
        "{finished:?}"
    );
}

#[test]
fn a_refused_piece_goes_to_this_mac_and_a_server_failure_does_not() {
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
