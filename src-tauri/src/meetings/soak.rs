//! Long meetings under forced kills, in child processes: the test binary
//! runs again as `soak_child`, once to record and once to process, and is
//! killed with SIGKILL mid-way as a crash or a power cut would stop the app.
//!
//! Recording drives the real coordinator and track writers, faster than
//! real time: a writer only reads each chunk's capture time. Processing
//! runs the launch path (`Library::open`, `waiting_for_processing`) and the
//! real `Processor`. Codex is a local server that answers each piece with
//! text tagged by the piece's audio, so a piece sent twice shows.
//!
//! `a_killed_meeting_is_recovered_and_processed_at_relaunch` runs in CI on
//! a few minutes of tones. `meetings_soak` runs three hours of two voices
//! made with `say` through the meetings helper and its models, and writes
//! `soak-report.md`. It takes about half an hour on the Mac:
//!
//! `cargo test --locked meetings_soak -- --ignored --nocapture`
//!
//! It never opens the app or plays a sound, and works in a temporary folder
//! with a clone of the models. `MURMUR_SOAK_MODELS` points at another models
//! folder, `MURMUR_SOAK_AUDIO` keeps the rendered dialogue in a folder of
//! its own, and `MURMUR_SOAK_CODEX=1` sends run A's pieces to the real Codex
//! session instead (about 36 requests of the user's quota).
//!
//! `echoes_without_the_models` processes the same dialogue, once through,
//! as a Mac without the models does, with a Codex that writes every echo
//! the microphone heard, and counts the echoes left under "Me" and the
//! user's words lost. It takes about a minute, in a temporary folder:
//!
//! `cargo test --locked echoes_without_the_models -- --ignored --nocapture`

use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::analysis::{AnalysisRequest, AnalyzeFuture, Analyzer, AnalyzerOutcome, HelperAnalyzer};
use super::capture::route::OutputRoute;
use super::capture::{Feeder, TrackWriter};
use super::coordinator::{
    self, CaptureBackend, Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier,
};
use super::helper;
use super::merge::{Track as Source, PLACEHOLDER};
use super::paths::{self, MIC_TRACK, SYSTEM_TRACK};
use super::processor::{MeetingProgress, ProcessEvents, Processor};
use super::store::{ChunkStatus, MeetingStatus, MeetingTracks, MeetingsStore, SpeakerAnalysis};
use super::summary::{CompleteFuture, Summarizer};
use super::tokens::{join, sentences, tokenize};
use super::track_audio;
use super::transcribe::{CodexTranscriber, RetryPolicy, TranscribeFuture, Transcriber};
use super::Library;
use crate::local_models::{manifest, Host, LocalModelStatus, LocalModels, PrepareFuture, Prepared};
use crate::settings::MeetingCaptureMode;
use crate::{OperationId, ProcessingOperation};

const CHILD_ENV: &str = "MURMUR_SOAK_CHILD";
/// Marks the lines a child prints for the parent, among the test output.
const LINE: &str = "SOAK ";
const RATE: u32 = 16_000;
/// The audio pushed to each track at a time.
const STEP_MS: u64 = 100;
/// How far ahead of a kill a recording slows to real time, so the kill finds
/// the writers as a crash would: a second of slack lets a writer that is
/// still behind catch up.
const REAL_TIME_BEFORE_KILL_MS: u64 = 3_000;
/// What a kill may lose of each track: the writer's buffer and queue.
const MAX_LOST_MS: u64 = 1_000;
/// The echo of the remote voice on the microphone, as speakers give it.
const ECHO_GAIN: f32 = 0.1;
const ECHO_DELAY_SAMPLES: usize = 640;
const GIB: u64 = 1 << 30;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Child {
    Record {
        data: PathBuf,
        dialogue: PathBuf,
        seconds: u64,
        /// Times real time.
        speed: u32,
        /// Audio past which recording goes at real time.
        real_time_from_ms: Option<u64>,
    },
    Process {
        data: PathBuf,
        /// The fake Codex and its session; `None` is the user's real one.
        codex: Option<(String, PathBuf)>,
        /// The models the helper runs with; `None` runs no analysis.
        models: Option<PathBuf>,
        /// Turns off what macOS's allocator keeps of freed large blocks (its
        /// large cache and deferred reclaim): they count in the footprint
        /// until the kernel takes them back. Off, the peak is what
        /// processing holds.
        release_freed: bool,
    },
}

/// The allocator settings `release_freed` stands for, read at launch.
const RELEASE_FREED: [(&str, &str); 2] =
    [("MallocLargeCache", "0"), ("MallocDeferredReclaim", "0")];

/// Printed after each step of recording.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
struct Recorded {
    id: i64,
    pushed_ms: u64,
}

/// Printed once processing is done.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Processed {
    recovered: Vec<i64>,
    waiting: Vec<i64>,
    seconds: f64,
    /// Peak memory of the processing process as macOS counts it (its
    /// footprint, which Activity Monitor shows), in bytes.
    footprint: u64,
    /// Peak resident memory of its largest child, the meetings helper.
    helper_rss: u64,
    /// The log lines with timings and results.
    log: Vec<String>,
}

#[test]
#[ignore = "run by the soak tests, in a child process"]
fn soak_child() {
    let Ok(child) = std::env::var(CHILD_ENV) else {
        return;
    };
    match serde_json::from_str(&child).unwrap() {
        Child::Record {
            data,
            dialogue,
            seconds,
            speed,
            real_time_from_ms,
        } => record(&data, &dialogue, seconds, speed, real_time_from_ms),
        Child::Process {
            data,
            codex,
            models,
            ..
        } => process(&data, codex, models),
    }
}

// ---- The recording child ----

/// Time that goes on as fast as the audio is pushed.
#[derive(Clone)]
struct PushedClock {
    origin: Instant,
    unix_ms: i64,
    pushed_us: Arc<AtomicU64>,
}

impl PushedClock {
    fn pushed(&self) -> Duration {
        Duration::from_micros(self.pushed_us.load(Ordering::SeqCst))
    }
}

impl Clock for PushedClock {
    fn now(&self) -> Instant {
        self.origin + self.pushed()
    }

    fn unix_ms(&self) -> i64 {
        self.unix_ms + self.pushed().as_millis() as i64
    }
}

struct WriterTrack {
    writer: TrackWriter,
}

impl coordinator::Track for WriterTrack {
    fn failed(&self) -> bool {
        self.writer.failed()
    }

    fn interrupted(&self) -> bool {
        false
    }

    fn dropped_ms(&self) -> u64 {
        self.writer.dropped_ms()
    }

    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.writer.finish(duration).map(|_| false)
    }
}

/// Track writers with no device: the soak pushes their audio.
#[derive(Default, Clone)]
struct PushedAudio(Arc<Mutex<Vec<(Source, Feeder)>>>);

impl PushedAudio {
    fn start(
        &self,
        source: Source,
        file: &'static str,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        let writer = TrackWriter::create(&dir.join(file), origin)?;
        self.0.lock().unwrap().push((source, writer.feeder()));
        Ok(Box::new(WriterTrack { writer }))
    }
}

impl CaptureBackend for PushedAudio {
    fn system_audio(&self) -> super::capture::system::SystemAudioSupport {
        super::capture::system::SystemAudioSupport::Available
    }

    fn start_mic(
        &self,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        self.start(Source::Mic, MIC_TRACK, dir, origin)
    }

    fn start_system(
        &self,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        self.start(Source::System, SYSTEM_TRACK, dir, origin)
    }

    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }

    fn microphone_denied(&self) -> bool {
        false
    }

    fn system_audio_denied(&self) -> bool {
        false
    }

    fn keep_awake(&self) -> Option<Box<dyn Send>> {
        None
    }

    /// The dialogue's microphone holds the echo built-in speakers give.
    fn output_route(&self) -> OutputRoute {
        OutputRoute::Speakers
    }
}

/// Alerts go to stderr: the report counts what they are about.
struct Unseen;

impl Notifier for Unseen {
    fn runtime_changed(&self, _: &MeetingRuntime) {}
    fn alert(&self, alert: MeetingAlert) {
        eprintln!("The soak recording raised {alert:?}");
    }
    fn recorded(&self, _: i64) {}
}

/// The dialogue played on a loop, a little later each time round so the
/// speaker models never see the same copy twice.
struct Looped {
    audio: Vec<f32>,
    round: usize,
    at: usize,
}

impl Looped {
    fn new(audio: Vec<f32>) -> Self {
        Self {
            audio,
            round: 0,
            at: 0,
        }
    }

    fn take(&mut self, count: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let gap = (RATE as usize / 1000) * (200 + 150 * (self.round % 3));
            if self.at >= self.audio.len() + gap {
                self.round += 1;
                self.at = 0;
                continue;
            }
            out.push(self.audio.get(self.at).copied().unwrap_or(0.0));
            self.at += 1;
        }
        out
    }
}

fn read_wav(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path)
        .unwrap_or_else(|error| panic!("Cannot read {}: {error}", path.display()));
    match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
            .collect(),
    }
}

fn record(data: &Path, dialogue: &Path, seconds: u64, speed: u32, real_time_from_ms: Option<u64>) {
    let library = Library::open(data).unwrap();
    let clock = PushedClock {
        origin: Instant::now(),
        unix_ms: chrono::Utc::now().timestamp_millis(),
        pushed_us: Arc::default(),
    };
    let audio = PushedAudio::default();
    let mut coordinator = Coordinator::new(
        audio.clone(),
        Unseen,
        clock.clone(),
        Arc::new(library.store),
        library.root,
    );
    let id = coordinator.start(MeetingCaptureMode::Both).unwrap();
    let mut tracks: Vec<(Feeder, Looped)> = std::mem::take(&mut *audio.0.lock().unwrap())
        .into_iter()
        .map(|(source, feeder)| {
            let file = match source {
                Source::Mic => MIC_TRACK,
                Source::System => SYSTEM_TRACK,
            };
            (feeder, Looped::new(read_wav(&dialogue.join(file))))
        })
        .collect();
    assert_eq!(tracks.len(), 2, "both tracks record");

    let total_ms = seconds * 1000;
    let step = (RATE as u64 * STEP_MS / 1000) as usize;
    let started = Instant::now();
    let mut paced = Duration::ZERO;
    let mut pushed_ms = 0;
    while pushed_ms < total_ms {
        pushed_ms += STEP_MS;
        let captured = clock.origin + Duration::from_millis(pushed_ms);
        for (feeder, audio) in &mut tracks {
            feeder.push_at(audio.take(step), RATE, captured);
        }
        clock.pushed_us.store(pushed_ms * 1000, Ordering::SeqCst);
        if pushed_ms % 1000 == 0 && pushed_ms < total_ms {
            assert!(coordinator.tick(id), "the meeting stopped recording");
        }
        println!(
            "{LINE}{}",
            serde_json::to_string(&Recorded { id, pushed_ms }).unwrap()
        );
        let real_time = real_time_from_ms.is_some_and(|from| pushed_ms >= from);
        paced += Duration::from_millis(STEP_MS) / if real_time { 1 } else { speed };
        if let Some(ahead) = paced.checked_sub(started.elapsed()) {
            std::thread::sleep(ahead);
        }
    }
    coordinator.stop().unwrap();
}

// ---- The processing child ----

/// Keeps the log lines processing writes, and echoes them to stderr.
struct KeptLog(Mutex<Vec<String>>);

impl log::Log for KeptLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info && metadata.target().starts_with("murmur")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let line = format!("{} {}", record.level(), record.args());
            eprintln!("{line}");
            self.0.lock().unwrap().push(line);
        }
    }

    fn flush(&self) {}
}

/// The peak resident memory of the largest child waited for, in bytes.
fn children_peak_rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is a valid rusage to write into.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) },
        0
    );
    // SAFETY: getrusage succeeded, and the struct was zeroed before.
    unsafe { usage.assume_init() }.ru_maxrss as u64
}

/// This process's peak footprint, in bytes. Unlike its resident size, it
/// leaves out the pages of the test binary itself, which are large in a
/// debug build.
fn peak_footprint() -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` is a valid rusage_info_v4 to write into, as the flavor says.
    let read = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V4,
            info.as_mut_ptr().cast(),
        )
    };
    assert_eq!(read, 0);
    // SAFETY: proc_pid_rusage succeeded, and the struct was zeroed before.
    unsafe { info.assume_init() }.ri_lifetime_max_phys_footprint
}

struct NoHost;

impl Host for NoHost {
    fn notify(&self, _: LocalModelStatus) {}
    fn remember_wanted(&self, _: bool) {}
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }
    fn prepare(&self, _: PathBuf, _: ProcessingOperation) -> PrepareFuture {
        Box::pin(async { Prepared::Failed })
    }
}

/// What the app does without the models: one speaker per track.
struct NoModels;

impl Analyzer for NoModels {
    fn analyze(&self, _: AnalysisRequest, _: ProcessingOperation) -> AnalyzeFuture {
        Box::pin(async { AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels) })
    }
}

struct CannedSummary;

impl Summarizer for CannedSummary {
    fn complete(&self, _: String, _: String, _: ProcessingOperation) -> CompleteFuture {
        Box::pin(async { Ok("# Soak meeting\nNotes.".to_string()) })
    }
}

struct NoEvents;

impl ProcessEvents for NoEvents {
    fn progress(&self, _: MeetingProgress) {}
    fn updated(&self, _: i64) {}
}

fn process(data: &Path, codex: Option<(String, PathBuf)>, models: Option<PathBuf>) {
    let kept: &'static KeptLog = Box::leak(Box::new(KeptLog(Mutex::default())));
    log::set_logger(kept).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    let started = Instant::now();
    let library = Library::open(data).unwrap();
    let store = Arc::new(library.store);
    let waiting = store.waiting_for_processing().unwrap();
    let analyzer: Arc<dyn Analyzer> = match models {
        Some(dir) => {
            let support = helper::support();
            let models = LocalModels::new(
                dir,
                manifest::this_mac(),
                support,
                manifest::HUGGING_FACE.to_string(),
                Arc::new(NoHost),
            );
            assert!(
                models.is_ready(),
                "the models are not ready: {:?}",
                models.status()
            );
            Arc::new(HelperAnalyzer { models, support })
        }
        None => Arc::new(NoModels),
    };
    let (url, auth_path) = match codex {
        Some((url, auth)) => (url, Some(auth)),
        None => (crate::codex_transcribe::TRANSCRIBE_URL.to_string(), None),
    };
    for id in &waiting {
        let processor = Processor {
            store: Arc::clone(&store),
            root: library.root.clone(),
            transcriber: Arc::new(CodexTranscriber {
                url: url.clone(),
                auth_path: auth_path.clone(),
            }),
            local: None,
            analyzer: Arc::clone(&analyzer),
            summarizer: Arc::new(CannedSummary),
            policy: RetryPolicy::default(),
            language: Some("fr".to_string()),
            events: Arc::new(NoEvents),
        };
        let operation = ProcessingOperation::new(OperationId(*id as u64));
        tauri::async_runtime::block_on(processor.process(*id, operation)).unwrap();
    }
    let log = kept
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| {
            ["timings:", "echo(es)", "Meeting analysis:", "processed:"]
                .iter()
                .any(|kept| line.contains(kept))
        })
        .cloned()
        .collect();
    let processed = Processed {
        recovered: library.recovered,
        waiting,
        seconds: started.elapsed().as_secs_f64(),
        footprint: peak_footprint(),
        helper_rss: children_peak_rss(),
        log,
    };
    println!("{LINE}{}", serde_json::to_string(&processed).unwrap());
}

// ---- The fake Codex ----

/// What the fake Codex answers, and when.
struct Answers {
    /// The hash of each request's audio, in order.
    requests: Mutex<Vec<u64>>,
    /// Requests past this many wait until it is lifted.
    hold_after: Mutex<Option<usize>>,
    lifted: Condvar,
    delay: Duration,
}

struct FakeCodex {
    url: String,
    answers: Arc<Answers>,
}

impl FakeCodex {
    fn start(delay: Duration, hold_after: Option<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/transcribe", listener.local_addr().unwrap());
        let answers = Arc::new(Answers {
            requests: Mutex::default(),
            hold_after: Mutex::new(hold_after),
            lifted: Condvar::new(),
            delay,
        });
        let serving = Arc::clone(&answers);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let answers = Arc::clone(&serving);
                std::thread::spawn(move || answer(stream, &answers));
            }
        });
        Self { url, answers }
    }

    fn requests(&self) -> Vec<u64> {
        self.answers.requests.lock().unwrap().clone()
    }

    fn lift_hold(&self) {
        *self.answers.hold_after.lock().unwrap() = None;
        self.answers.lifted.notify_all();
    }
}

/// The body of an HTTP request, sized or chunked.
fn read_body(reader: &mut BufReader<TcpStream>) -> Option<Vec<u8>> {
    let (mut length, mut chunked) = (0, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end().to_ascii_lowercase();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("content-length:") {
            length = value.trim().parse().ok()?;
        }
        chunked |= line.starts_with("transfer-encoding:") && line.contains("chunked");
    }
    let mut body = Vec::new();
    if !chunked {
        body.resize(length, 0);
        reader.read_exact(&mut body).ok()?;
        return Some(body);
    }
    loop {
        let mut size = String::new();
        reader.read_line(&mut size).ok()?;
        let size = usize::from_str_radix(size.trim(), 16).ok()?;
        let mut data = vec![0; size + 2];
        reader.read_exact(&mut data).ok()?;
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(&data[..size]);
    }
}

fn answer(stream: TcpStream, answers: &Answers) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let Some(body) = read_body(&mut reader) else {
        return;
    };
    let wav = uploaded_wav(&body).expect("a transcription request uploads a WAV");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    wav.hash(&mut hasher);
    let piece = hasher.finish();
    let number = {
        let mut requests = answers.requests.lock().unwrap();
        requests.push(piece);
        requests.len() - 1
    };
    drop(
        answers
            .lifted
            .wait_while(answers.hold_after.lock().unwrap(), |hold| {
                hold.is_some_and(|after| number >= after)
            })
            .unwrap(),
    );
    std::thread::sleep(answers.delay);
    let text = serde_json::json!({ "text": piece_text(piece, wav.len()) }).to_string();
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
}

/// The WAV a transcription request uploads, which identifies the piece. The
/// rest of the body differs at each request: reqwest draws a new multipart
/// boundary for each form.
fn uploaded_wav(body: &[u8]) -> Option<&[u8]> {
    let start = body.windows(4).position(|window| window == b"RIFF")?;
    let size: [u8; 4] = body.get(start + 4..start + 8)?.try_into().ok()?;
    body.get(start..start + 8 + u32::from_le_bytes(size) as usize)
}

/// About as many words as a track of half a dialogue holds in a piece of
/// `bytes` of 16-bit audio, tagged with the piece.
fn piece_text(piece: u64, bytes: usize) -> String {
    let words = bytes / 32_000 * 13 / 10;
    let mut text = format!("Piece {piece:016x}.");
    let mut count = 0;
    for (_, line) in LINES.iter().cycle().skip(piece as usize % LINES.len()) {
        if count >= words {
            break;
        }
        text.push(' ');
        text.push_str(line);
        count += line.split_whitespace().count();
    }
    text
}

/// The piece a transcribed chunk's text was the answer for.
fn piece_of(text: &str) -> Option<u64> {
    let tag = text.strip_prefix("Piece ")?.get(..16)?;
    u64::from_str_radix(tag, 16).ok()
}

#[test]
fn the_fake_codex_knows_a_piece_by_its_audio() {
    let work = tempfile::tempdir().unwrap();
    let codex = FakeCodex::start(Duration::ZERO, None);
    let auth = auth_file(work.path());
    let send = |samples: &[f32]| {
        let operation = ProcessingOperation::new(OperationId(1));
        let text = tauri::async_runtime::block_on(crate::codex_transcribe::transcribe_at(
            &codex.url,
            Some(&auth),
            samples,
            Some("fr"),
            1,
            &operation,
        ))
        .unwrap();
        piece_of(&text).unwrap()
    };
    let tone: Vec<f32> = (0..RATE as usize)
        .map(|sample| (sample as f32 * 0.07).sin() * 0.4)
        .collect();
    let quieter: Vec<f32> = tone.iter().map(|sample| sample / 2.0).collect();

    let piece = send(&tone);
    assert_eq!(send(&tone), piece, "the same audio sent again");
    assert_ne!(send(&quieter), piece, "other audio");
    assert_eq!(codex.requests().len(), 3);
}

fn auth_file(dir: &Path) -> PathBuf {
    let path = dir.join("auth.json");
    let auth = serde_json::json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "access_token": "soak-token",
            "refresh_token": "soak-refresh",
            "account_id": "soak-account"
        },
        "last_refresh": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(&path, auth.to_string()).unwrap();
    path
}

// ---- The parent ----

/// A child run of the test binary.
struct Running {
    child: std::process::Child,
    lines: Receiver<String>,
}

fn spawn(exe: &Path, child: &Child) -> Running {
    let mut command = Command::new(exe);
    command
        .args([
            "meetings::soak::soak_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, serde_json::to_string(child).unwrap())
        .stdout(Stdio::piped());
    if matches!(
        child,
        Child::Process {
            release_freed: true,
            ..
        }
    ) {
        command.envs(RELEASE_FREED);
    }
    let mut process = command.spawn().unwrap();
    let stdout = process.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            // The first follows libtest's "test soak_child ... " line.
            if let Some((_, json)) = line.split_once(LINE) {
                if sender.send(json.to_string()).is_err() {
                    return;
                }
            }
        }
    });
    Running {
        child: process,
        lines,
    }
}

impl Running {
    /// Waits for the child to end well, and returns the last line it printed.
    fn finish<T: serde::de::DeserializeOwned>(mut self) -> T {
        let status = self.child.wait().unwrap();
        assert!(status.success(), "the soak child failed: {status}");
        let last = self
            .lines
            .iter()
            .last()
            .expect("the soak child printed nothing");
        serde_json::from_str(&last).unwrap()
    }

    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

struct Recording {
    id: i64,
    /// Audio pushed to each track before it ended.
    pushed_ms: u64,
}

/// Records `seconds` of the dialogue in `data`, or until the microphone
/// track holds `kill_at_ms` on disk, where the recording is killed.
fn record_in(
    exe: &Path,
    data: &Path,
    dialogue: &Path,
    seconds: u64,
    kill_at_ms: Option<u64>,
) -> Recording {
    let mut running = spawn(
        exe,
        &Child::Record {
            data: data.to_path_buf(),
            dialogue: dialogue.to_path_buf(),
            seconds,
            speed: 100,
            real_time_from_ms: kill_at_ms.map(|at| at.saturating_sub(REAL_TIME_BEFORE_KILL_MS)),
        },
    );
    let Some(kill_at_ms) = kill_at_ms else {
        let last: Recorded = running.finish();
        assert_eq!(last.pushed_ms, seconds * 1000);
        return Recording {
            id: last.id,
            pushed_ms: last.pushed_ms,
        };
    };
    let mut last: Option<Recorded> = None;
    loop {
        match running.lines.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => last = Some(serde_json::from_str(&line).unwrap()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("the recording ended before its kill"),
        }
        let Some(Recorded { id, .. }) = last else {
            continue;
        };
        let mic = paths::meeting_dir(&data.join("meetings"), id).join(MIC_TRACK);
        let on_disk_ms =
            std::fs::metadata(&mic).map_or(0, |file| file.len().saturating_sub(44) / 32);
        if on_disk_ms >= kill_at_ms {
            break;
        }
    }
    running.kill();
    let last = running
        .lines
        .iter()
        .map(|line| serde_json::from_str::<Recorded>(&line).unwrap())
        .last()
        .or(last)
        .unwrap();
    Recording {
        id: last.id,
        pushed_ms: last.pushed_ms,
    }
}

/// Processes the library in `data` until `done_share` of meeting `id`'s
/// pieces with speech are transcribed, and kills it there. Returns the
/// pieces it saved as transcribed.
fn process_until_killed(
    exe: &Path,
    child: &Child,
    store: &MeetingsStore,
    id: i64,
    done_share: f64,
) -> Vec<u64> {
    let mut running = spawn(exe, child);
    let done = |store: &MeetingsStore| -> Option<Vec<u64>> {
        let (speech, done) = transcribed(store, id)?;
        (speech > 0 && done.len() as f64 >= (speech as f64 * done_share).max(1.0)).then_some(done)
    };
    loop {
        assert!(
            running.child.try_wait().unwrap().is_none(),
            "processing ended before its kill"
        );
        if done(store).is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    running.kill();
    done(store).expect("the transcribed pieces stay saved")
}

/// How many pieces of meeting `id` hold speech, and the audio of those saved
/// as transcribed.
fn transcribed(store: &MeetingsStore, id: i64) -> Option<(usize, Vec<u64>)> {
    let chunks = store.chunks(id).ok()?;
    let speech = chunks
        .iter()
        .filter(|chunk| chunk.status != ChunkStatus::Silent)
        .count();
    let done = chunks
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Done)
        .filter_map(|chunk| piece_of(chunk.text.as_deref()?))
        .collect();
    Some((speech, done))
}

/// How many times a relaunch sent Codex a piece already transcribed before
/// it: `sent` sends of the audio in `done_before`, beyond the pieces of that
/// audio still to do. Counted per audio, as two pieces can hold the same.
fn sent_again(done_before: &[u64], sent: &[u64], done_after: &[u64]) -> usize {
    let count = |pieces: &[u64], piece: u64| pieces.iter().filter(|&&other| other == piece).count();
    done_before
        .iter()
        .copied()
        .collect::<HashSet<u64>>()
        .into_iter()
        .map(|piece| {
            (count(done_before, piece) + count(sent, piece))
                .saturating_sub(count(done_after, piece))
        })
        .sum()
}

#[test]
fn a_piece_sent_again_after_its_transcription_is_counted() {
    // Pieces 1 and 2 were done, 3 and 4 were left.
    assert_eq!(sent_again(&[1, 2], &[3, 4], &[1, 2, 3, 4]), 0);
    assert_eq!(sent_again(&[1, 2], &[3, 1, 4], &[1, 2, 3, 4]), 1);
    // Two pieces of the same audio, one done before: the other is still due.
    assert_eq!(sent_again(&[7], &[7], &[7, 7]), 0);
    assert_eq!(sent_again(&[7], &[7, 7], &[7, 7]), 1);
}

/// What one meeting ended as.
struct Outcome {
    status: MeetingStatus,
    duration_ms: i64,
    /// Bubbles per speaker key.
    turns: BTreeMap<String, usize>,
    analysis: Option<SpeakerAnalysis>,
}

fn outcome(store: &MeetingsStore, id: i64) -> Outcome {
    let meeting = store.get(id).unwrap().unwrap();
    let mut turns = BTreeMap::new();
    for turn in &meeting.turns {
        *turns.entry(turn.speaker_key.clone()).or_insert(0) += 1;
    }
    Outcome {
        status: meeting.status,
        duration_ms: meeting.duration_ms,
        turns,
        analysis: meeting.speaker_analysis,
    }
}

impl Outcome {
    /// The user and at least one remote voice have bubbles.
    fn both_speakers(&self) -> bool {
        self.turns.contains_key("me")
            && self
                .turns
                .keys()
                .any(|key| key != "me" && key != PLACEHOLDER)
    }
}

/// The samples each track of a recovered meeting holds, in milliseconds.
fn track_ms(data: &Path, id: i64) -> [u64; 2] {
    let dir = paths::meeting_dir(&data.join("meetings"), id);
    [MIC_TRACK, SYSTEM_TRACK].map(|file| {
        let reader = hound::WavReader::open(dir.join(file)).unwrap();
        u64::from(reader.duration()) * 1000 / u64::from(RATE)
    })
}

/// The two tracks of a dialogue, written as `mic.wav` and `system.wav`:
/// voice A on the microphone, voice B on system audio, and B's echo on the
/// microphone, as built-in speakers give it. Returns where each turn starts,
/// in samples.
fn write_dialogue(dir: &Path, turns: &[(bool, Vec<f32>)]) -> Vec<usize> {
    let pause = |index: usize| RATE as usize / 1000 * (500 + 250 * (index % 3));
    let length: usize = turns
        .iter()
        .enumerate()
        .map(|(index, (_, clip))| clip.len() + pause(index))
        .sum::<usize>()
        + ECHO_DELAY_SAMPLES;
    let (mut mic, mut system) = (vec![0f32; length], vec![0f32; length]);
    let mut at = 0;
    let mut starts = Vec::with_capacity(turns.len());
    for (index, (remote, clip)) in turns.iter().enumerate() {
        starts.push(at);
        for (offset, sample) in clip.iter().enumerate() {
            if *remote {
                system[at + offset] += sample;
                mic[at + offset + ECHO_DELAY_SAMPLES] += sample * ECHO_GAIN;
            } else {
                mic[at + offset] += sample;
            }
        }
        at += clip.len() + pause(index);
    }
    std::fs::create_dir_all(dir).unwrap();
    for (file, track) in [(MIC_TRACK, mic), (SYSTEM_TRACK, system)] {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(dir.join(file), spec).unwrap();
        for sample in track {
            let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }
    starts
}

/// Alternating tones that read as speech, for the quick variant.
fn tone_dialogue(dir: &Path, seconds: usize) {
    let turns: Vec<(bool, Vec<f32>)> = (0..seconds / 5)
        .map(|index| {
            let pitch = if index % 2 == 0 { 0.07 } else { 0.11 };
            let clip = (0..4 * RATE as usize)
                .map(|sample| (sample as f32 * pitch).sin() * 0.4)
                .collect();
            (index % 2 == 1, clip)
        })
        .collect();
    write_dialogue(dir, &turns);
}

#[test]
fn a_killed_meeting_is_recovered_and_processed_at_relaunch() {
    let work = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let dialogue = work.path().join("dialogue");
    tone_dialogue(&dialogue, 60);
    let data = work.path().join("data");

    // Killed half way through three minutes.
    let recording = record_in(&exe, &data, &dialogue, 180, Some(90_000));
    let id = recording.id;
    assert!(recording.pushed_ms < 180_000, "the kill came too late");

    // The relaunch processes it, and is killed in turn once one piece is
    // transcribed: Codex holds the others.
    let (store, _) = MeetingsStore::open(data.join("meetings.db"), chrono::Utc::now());
    let codex = FakeCodex::start(Duration::ZERO, Some(1));
    let child = Child::Process {
        data: data.clone(),
        codex: Some((codex.url.clone(), auth_file(work.path()))),
        models: None,
        release_freed: false,
    };
    let done = process_until_killed(&exe, &child, &store, id, 0.0);
    for (track, ms) in track_ms(&data, id).into_iter().enumerate() {
        assert!(
            ms + MAX_LOST_MS >= recording.pushed_ms,
            "track {track} kept {ms} ms of {} ms",
            recording.pushed_ms
        );
    }
    assert_eq!(done.len(), 1);

    // The next launch sends only what is left.
    codex.lift_hold();
    let sent_before = codex.requests().len();
    let processed: Processed = spawn(&exe, &child).finish();
    assert_eq!(
        processed.waiting,
        [id],
        "the interrupted meeting is processed"
    );
    let sent = &codex.requests()[sent_before..];
    let (_, done_after) = transcribed(&store, id).unwrap();
    assert_eq!(
        sent_again(&done, sent, &done_after),
        0,
        "a done piece was sent again"
    );
    let meeting = outcome(&store, id);
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert!(meeting.both_speakers(), "{:?}", meeting.turns);
}

// ---- The three-hour soak ----

/// The voices of the dialogue: A is the user, B the remote side.
const VOICE_A: &str = "Thomas";
const VOICE_B: &str = "Amélie";

/// The dialogue, A then B in turn. Said three times at different rates, it
/// lasts about ten minutes.
const LINES: &[(bool, &str)] = &[
    (false, "Bonjour à tous, merci d'être là ce matin pour faire le point sur le projet."),
    (true, "Bonjour, merci pour l'invitation. J'ai préparé les chiffres du dernier trimestre."),
    (false, "Parfait. Commençons par le calendrier, où en est la livraison de la nouvelle version ?"),
    (true, "La version est prête à quatre-vingts pour cent. Il reste les tests sur les anciens ordinateurs."),
    (false, "Combien de temps faut-il prévoir pour ces tests, à ton avis ?"),
    (true, "Je dirais deux semaines, si l'équipe de validation peut commencer lundi prochain."),
    (false, "D'accord. Et du côté du budget, est-ce qu'on reste dans l'enveloppe prévue ?"),
    (true, "Oui, nous avons dépensé un peu moins que prévu sur les serveurs ce trimestre."),
    (false, "Très bien. Est-ce qu'on peut réutiliser cette marge pour embaucher un stagiaire ?"),
    (true, "C'est possible, mais il faut d'abord valider la demande avec les ressources humaines."),
    (false, "Je m'en occupe cette semaine. Passons aux retours des clients sur la dernière version."),
    (true, "Les retours sont plutôt bons. Les utilisateurs apprécient surtout la recherche plus rapide."),
    (false, "Et les plaintes ? J'ai vu passer plusieurs messages sur la synchronisation."),
    (true, "Oui, la synchronisation échoue parfois quand la connexion est lente. Nous avons trouvé la cause."),
    (false, "Quelle est la cause exactement, et quand pouvons-nous livrer une correction ?"),
    (true, "Un délai trop court sur les requêtes. La correction partira avec la prochaine mise à jour."),
    (false, "Bien. Il faudra prévenir le support pour qu'ils puissent répondre aux clients."),
    (true, "Je leur enverrai une note cet après-midi avec les détails et les dates."),
    (false, "Merci. Parlons maintenant de la conférence du mois prochain à Lyon."),
    (true, "Nous avons un stand et deux présentations. Il manque encore quelqu'un pour la seconde."),
    (false, "Je peux faire la seconde présentation si tu m'envoies les diapositives avant vendredi."),
    (true, "Entendu, je te les envoie jeudi soir au plus tard, avec mes notes."),
    (false, "Est-ce qu'on a prévu des démonstrations sur le stand, ou seulement des affiches ?"),
    (true, "Deux ordinateurs de démonstration, et une tablette pour inscrire les visiteurs intéressés."),
    (false, "Il faudra vérifier que la connexion du salon permet de faire la démonstration en direct."),
    (true, "Bonne remarque. Je demanderai aux organisateurs le débit garanti sur les stands."),
    (false, "Autre sujet, la migration de la base de données. Où en sommes-nous ?"),
    (true, "La copie de test fonctionne. Nous voulons migrer la production un dimanche matin."),
    (false, "Combien de temps le service sera-t-il coupé pendant la migration ?"),
    (true, "Environ deux heures, et nous gardons l'ancienne base pendant un mois au cas où."),
    (false, "Ça me semble raisonnable. Il faudra prévenir les clients une semaine avant."),
    (true, "Je préparerai le message et je te le soumets avant de l'envoyer."),
    (false, "Parfait. Est-ce qu'il y a d'autres points que tu voulais aborder aujourd'hui ?"),
    (true, "Juste un, les congés d'été. Il faudrait que chacun donne ses dates avant la fin du mois."),
    (false, "Je relaierai la demande à l'équipe. On garde au moins deux personnes en août."),
    (true, "C'est noté. De mon côté, je serai absente les deux premières semaines de juillet."),
    (false, "Merci pour toutes ces informations. On se retrouve la semaine prochaine, même heure ?"),
    (true, "Oui, même heure. Bonne journée à tous, et merci pour cette réunion."),
];

/// Says the dialogue into `dir`, unless it is there already. `say` writes
/// to files only: nothing plays.
fn say_dialogue(dir: &Path) {
    if dir.join(MIC_TRACK).is_file() && dir.join(SYSTEM_TRACK).is_file() {
        return;
    }
    let turns: Vec<(bool, Vec<f32>)> = said_lines()
        .into_iter()
        .map(|((remote, _), clip)| (remote, clip))
        .collect();
    write_dialogue(dir, &turns);
}

/// Each line of the dialogue, said three times at different rates, with
/// its audio.
fn said_lines() -> Vec<((bool, &'static str), Vec<f32>)> {
    let clips = tempfile::tempdir().unwrap();
    let mut said = Vec::new();
    for (round, rate) in [175, 195, 215].into_iter().enumerate() {
        for (index, &(remote, line)) in LINES.iter().enumerate() {
            let clip = clips.path().join(format!("{round}-{index}.wav"));
            let status = Command::new("say")
                .args(["-v", if remote { VOICE_B } else { VOICE_A }])
                .args(["-r", &rate.to_string(), "-o"])
                .arg(&clip)
                .args(["--file-format=WAVE", "--data-format=LEF32@16000", line])
                .status()
                .unwrap();
            assert!(status.success(), "say failed: {status}");
            said.push(((remote, line), read_wav(&clip)));
        }
    }
    said
}

/// The analysis stage's time, read from the timings line.
fn stage_seconds(processed: &Processed, stage: &str) -> Option<f64> {
    let line = processed
        .log
        .iter()
        .find(|line| line.contains("timings:"))?;
    let (_, after) = line.split_once(&format!("{stage} "))?;
    after.split_once(" s")?.0.parse().ok()
}

fn echo_removals(processed: &Processed) -> usize {
    processed
        .log
        .iter()
        .filter_map(|line| {
            let (_, after) = line.split_once("Removed ")?;
            after.split_once(" echo")?.0.parse::<usize>().ok()
        })
        .sum()
}

fn sh(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / GIB as f64)
}

fn minutes(ms: u64) -> String {
    format!(
        "{}:{:02}:{:02}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60
    )
}

/// One check of the report, and whether it passed.
struct Check {
    what: String,
    passed: bool,
}

fn check(what: impl Into<String>, passed: bool) -> Check {
    Check {
        what: what.into(),
        passed,
    }
}

#[test]
#[ignore = "three hours of audio through the real models: about half an hour"]
fn meetings_soak() {
    const THREE_HOURS: u64 = 3 * 3600;
    let work = tempfile::tempdir().unwrap();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    // The test binary runs from a folder with the helper next to it, where
    // the app looks for it.
    let bin = work.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("murmur-soak");
    std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    let built = manifest_dir.join("binaries").join(format!(
        "{}-{}-apple-darwin",
        helper::HELPER_NAME,
        std::env::consts::ARCH
    ));
    std::fs::copy(&built, bin.join(helper::HELPER_NAME))
        .unwrap_or_else(|error| panic!("No helper at {}: {error}", built.display()));

    // A clone of the models: APFS shares their blocks, and the original is
    // only read.
    let models_from = std::env::var_os("MURMUR_SOAK_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs_home().join("Library/Application Support/com.dailyxplorer.murmur/meetings/models")
        });
    let models = work.path().join("models");
    let cloned = Command::new("cp")
        .arg("-Rc")
        .arg(&models_from)
        .arg(&models)
        .status()
        .unwrap();
    assert!(cloned.success(), "cannot clone {}", models_from.display());

    let dialogue = std::env::var_os("MURMUR_SOAK_AUDIO")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.path().join("dialogue"));
    say_dialogue(&dialogue);
    let dialogue_ms = u64::from(
        hound::WavReader::open(dialogue.join(MIC_TRACK))
            .unwrap()
            .duration(),
    ) * 1000
        / u64::from(RATE);

    let codex = FakeCodex::start(Duration::from_secs(2), None);
    let fake = Some((codex.url.clone(), auth_file(work.path())));
    let real_codex = std::env::var("MURMUR_SOAK_CODEX").as_deref() == Ok("1");

    // Run A: three hours, recorded and processed without a stop.
    let data_a = work.path().join("a");
    let started = Instant::now();
    let a = record_in(&exe, &data_a, &dialogue, THREE_HOURS, None);
    let recorded_a = started.elapsed();
    // Processed twice, from clones of the library: as the app runs, and
    // with freed memory handed back at once, which leaves what it holds.
    let held_a = work.path().join("a-held");
    let cloned = Command::new("cp")
        .arg("-Rc")
        .arg(&data_a)
        .arg(&held_a)
        .status()
        .unwrap();
    assert!(cloned.success(), "cannot clone run A");
    let (store_a, _) = MeetingsStore::open(data_a.join("meetings.db"), chrono::Utc::now());
    let dropped_a = store_a.dropped_ms(a.id).unwrap();
    let processed_a: Processed = spawn(
        &exe,
        &Child::Process {
            data: data_a.clone(),
            codex: if real_codex { None } else { fake.clone() },
            models: Some(models.clone()),
            release_freed: false,
        },
    )
    .finish();
    let outcome_a = outcome(&store_a, a.id);
    let processed_held: Processed = spawn(
        &exe,
        &Child::Process {
            data: held_a,
            codex: fake.clone(),
            models: Some(models.clone()),
            release_freed: true,
        },
    )
    .finish();

    // Run B: killed at an hour and a half of recording.
    let data_b = work.path().join("b");
    let b = record_in(
        &exe,
        &data_b,
        &dialogue,
        THREE_HOURS,
        Some(THREE_HOURS * 500),
    );
    let (store_b, _) = MeetingsStore::open(data_b.join("meetings.db"), chrono::Utc::now());

    // Run C: the relaunch that processes it is killed half way through the
    // pieces, and the next one finishes.
    let child_b = Child::Process {
        data: data_b.clone(),
        codex: fake.clone(),
        models: Some(models.clone()),
        release_freed: false,
    };
    let done_c = process_until_killed(&exe, &child_b, &store_b, b.id, 0.5);
    let kept_b = track_ms(&data_b, b.id);
    let sent_before = codex.requests().len();
    let processed_c: Processed = spawn(&exe, &child_b).finish();
    let sent_c = codex.requests()[sent_before..].to_vec();
    let (_, done_after_c) = transcribed(&store_b, b.id).unwrap();
    let resent = sent_again(&done_c, &sent_c, &done_after_c);
    let outcome_c = outcome(&store_b, b.id);

    let analysis_a = stage_seconds(&processed_a, "analysis");
    let analysis_c = stage_seconds(&processed_c, "analysis");
    let checks = [
        check("A is ready", outcome_a.status == MeetingStatus::Ready),
        check(
            "A: processing holds under 1 GiB",
            processed_held.footprint < GIB,
        ),
        check("A: helper under 3 GiB", processed_a.helper_rss < 3 * GIB),
        check(
            "A: analysis under 10 min",
            analysis_a.is_some_and(|seconds| seconds < 600.0),
        ),
        check("A: both speakers present", outcome_a.both_speakers()),
        check("A: no audio dropped", dropped_a == 0),
        check(
            "B: each track kept all but 1 s of what was pushed",
            kept_b.iter().all(|ms| ms + MAX_LOST_MS >= b.pushed_ms),
        ),
        check(
            "C: the interrupted meeting is processed at relaunch",
            processed_c.waiting.contains(&b.id),
        ),
        check("C: no transcribed piece sent again", resent == 0),
        check("C is ready", outcome_c.status == MeetingStatus::Ready),
        check("C: helper under 3 GiB", processed_c.helper_rss < 3 * GIB),
        check("C: both speakers present", outcome_c.both_speakers()),
    ];

    let mut report = String::new();
    let mut line = |text: String| {
        report.push_str(&text);
        report.push('\n');
    };
    line("# Meetings soak report".into());
    line(String::new());
    line("Written by `meetings_soak` (`src-tauri/src/meetings/soak.rs`). Run it on a Mac with the models downloaded:".into());
    line(String::new());
    line("```sh".into());
    line("cd src-tauri && cargo test --locked meetings_soak -- --ignored --nocapture".into());
    line("```".into());
    line(String::new());
    line(format!(
        "- Machine: {}, {} GB RAM, macOS {}",
        sh("sysctl", &["-n", "machdep.cpu.brand_string"]),
        sh("sysctl", &["-n", "hw.memsize"])
            .parse::<u64>()
            .map_or("unknown".into(), |bytes| (bytes / 1_000_000_000)
                .to_string()),
        sh("sw_vers", &["-productVersion"]),
    ));
    line(format!(
        "- Commit: {}",
        std::env::var("MURMUR_SOAK_COMMIT").unwrap_or_else(|_| sh(
            "git",
            &[
                "-C",
                env!("CARGO_MANIFEST_DIR"),
                "rev-parse",
                "--short",
                "HEAD"
            ]
        ))
    ));
    line(
        "- Build: debug test binary; the meetings helper is the release build `build.rs` makes"
            .into(),
    );
    line(format!(
        "- Audio: {} of French dialogue said by `say` ({VOICE_A} on the microphone, {VOICE_B} on system audio and at -20 dB, 40 ms late, on the microphone), looped to 3 hours with 200 to 500 ms more silence each time round",
        minutes(dialogue_ms)
    ));
    line(format!(
        "- Codex: {}; the summary is canned",
        if real_codex {
            "the real session for run A, the fake one for C"
        } else {
            "a local fake that answers each piece after 2 s with about as many words as it holds, so alignment and echo removal work on text that does not match the audio; it tags each answer with a hash of the piece's WAV, which tells the pieces sent again"
        }
    ));
    line(String::new());
    line("## Runs".into());
    line(String::new());
    line("| | A: 3 h, no stop | C: B's meeting after both kills |".into());
    line("|---|---|---|".into());
    let both = |label: &str, a: String, c: String| format!("| {label} | {a} | {c} |");
    line(both(
        "Audio processed",
        minutes(outcome_a.duration_ms.max(0) as u64),
        minutes(outcome_c.duration_ms.max(0) as u64),
    ));
    line(both(
        "Peak footprint of processing, as Activity Monitor shows it",
        gib(processed_a.footprint),
        gib(processed_c.footprint),
    ));
    line(both(
        "Peak footprint of processing, freed memory handed back at once",
        gib(processed_held.footprint),
        "not run".into(),
    ));
    line(both(
        "Peak resident memory, meetings helper",
        gib(processed_a.helper_rss),
        gib(processed_c.helper_rss),
    ));
    let seconds = |value: Option<f64>| value.map_or("unknown".into(), |s| format!("{s:.1} s"));
    line(both(
        "Analysis (helper)",
        seconds(analysis_a),
        seconds(analysis_c),
    ));
    line(both(
        "Transcription",
        seconds(stage_seconds(&processed_a, "transcription")),
        seconds(stage_seconds(&processed_c, "transcription")),
    ));
    line(both(
        "Aligning",
        seconds(stage_seconds(&processed_a, "aligning")),
        seconds(stage_seconds(&processed_c, "aligning")),
    ));
    line(both(
        "Compression",
        seconds(stage_seconds(&processed_a, "compression")),
        seconds(stage_seconds(&processed_c, "compression")),
    ));
    line(both(
        "Whole relaunch",
        format!("{:.1} s", processed_a.seconds),
        format!("{:.1} s", processed_c.seconds),
    ));
    line(both(
        "Speakers",
        format!("{:?}", outcome_a.analysis),
        format!("{:?}", outcome_c.analysis),
    ));
    line(both(
        "Bubbles per speaker",
        format!("{:?}", outcome_a.turns),
        format!("{:?}", outcome_c.turns),
    ));
    line(both(
        "Echoes removed",
        echo_removals(&processed_a).to_string(),
        echo_removals(&processed_c).to_string(),
    ));
    line(String::new());
    line(format!(
        "Run A recorded 3 hours in {:.0} s and dropped {dropped_a} ms of audio.",
        recorded_a.as_secs_f64()
    ));
    line(String::new());
    line(format!(
        "Run B was killed (SIGKILL) at {} of recording, after {} ms had been pushed to each track. At the relaunch the microphone track held {} ms and system audio {} ms.",
        minutes(b.pushed_ms),
        b.pushed_ms,
        kept_b[0],
        kept_b[1]
    ));
    line(String::new());
    line(format!(
        "Run C: the first relaunch was killed once {} pieces were transcribed. The next one sent {} pieces, {resent} of them already transcribed.",
        done_c.len(),
        sent_c.len()
    ));
    line(String::new());
    line("Memory: each piece sent to Codex takes short-lived buffers (its samples and copies of them, the WAV and the upload body: over 100 MB for 10 minutes), freed once the piece is done. macOS's allocator keeps freed blocks that large in the footprint (its large cache and deferred reclaim) until the kernel takes them back, so the first row grows with the number of pieces and falls again after transcription. The second row is the same processing of run A run with `MallocLargeCache=0 MallocDeferredReclaim=0`, which hands freed memory back at once: what processing holds, and what the 1 GiB threshold checks.".into());
    line(String::new());
    line("Timings as logged:".into());
    line(String::new());
    for (run, processed) in [
        ("A", &processed_a),
        ("A, freed memory handed back", &processed_held),
        ("C", &processed_c),
    ] {
        for logged in processed
            .log
            .iter()
            .filter(|line| line.contains("timings:"))
        {
            line(format!("- {run}: `{logged}`"));
        }
    }
    line(String::new());
    line("## Thresholds".into());
    line(String::new());
    for check in &checks {
        line(format!(
            "- [{}] {}",
            if check.passed { "x" } else { " " },
            check.what
        ));
    }
    line(String::new());
    let failed: Vec<&str> = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.what.as_str())
        .collect();
    line(if failed.is_empty() {
        "Every threshold passes: the helper keeps reading each track whole, with no windows.".into()
    } else {
        format!("Failed: {}.", failed.join("; "))
    });

    let path = std::env::var_os("MURMUR_SOAK_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("src/meetings/soak-report.md"));
    std::fs::write(&path, &report).unwrap();
    println!("{report}");
    assert!(failed.is_empty(), "soak thresholds failed: {failed:?}");
}

// ---- Echoes without the on-device models ----

/// Codex as a script: each piece is answered with the lines said in it, the
/// echoes the microphone heard included, found by where its audio sits in
/// the track.
struct ScriptedCodex {
    tracks: Vec<ScriptedTrack>,
}

struct ScriptedTrack {
    samples: Vec<f32>,
    /// Where each line said on the track starts, in samples.
    lines: Vec<(usize, &'static str)>,
}

impl ScriptedCodex {
    fn answer(&self, piece: &[f32]) -> String {
        let Some(first) = piece.iter().position(|sample| *sample != 0.0) else {
            return String::new();
        };
        let probe = &piece[first..(first + 256).min(piece.len())];
        for ScriptedTrack { samples, lines } in &self.tracks {
            if let Some(at) = samples
                .windows(probe.len())
                .position(|window| window == probe)
            {
                let start = at - first;
                return lines
                    .iter()
                    .filter(|(line_at, _)| (start..start + piece.len()).contains(line_at))
                    .map(|(_, line)| *line)
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
        panic!("a piece is in neither track");
    }
}

impl Transcriber for ScriptedCodex {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        _: Option<String>,
        _: ProcessingOperation,
    ) -> TranscribeFuture {
        let text = self.answer(&samples);
        Box::pin(async move { Ok(text) })
    }
}

/// How often the words of `line` come in a row in `keys`, a run of word
/// keys with a space on either side.
fn said_in(keys: &str, line: &str) -> usize {
    let line: Vec<String> = tokenize(line)
        .into_iter()
        .map(|token| token.key)
        .filter(|key| !key.is_empty())
        .collect();
    keys.matches(&format!(" {} ", line.join(" "))).count()
}

/// The dialogue of the soak, processed as on a Mac without the on-device
/// models, with Codex writing every echo the microphone heard: the echoes
/// left under "Me" against the same meeting recorded on headphones, where
/// nothing is removed, and the user's own words lost, which must be none.
#[test]
#[ignore = "renders the dialogue with `say` (nothing plays): about a minute"]
fn echoes_without_the_models() {
    let work = tempfile::tempdir().unwrap();
    let dialogue = work.path().join("dialogue");
    let said = said_lines();
    let turns: Vec<(bool, Vec<f32>)> = said
        .iter()
        .map(|((remote, _), clip)| (*remote, clip.clone()))
        .collect();
    let starts = write_dialogue(&dialogue, &turns);
    let (mut mic_lines, mut system_lines) = (Vec::new(), Vec::new());
    for (((remote, line), _), &start) in said.iter().zip(&starts) {
        if *remote {
            system_lines.push((start, *line));
            mic_lines.push((start + ECHO_DELAY_SAMPLES, *line));
        } else {
            mic_lines.push((start, *line));
        }
    }
    mic_lines.sort();
    let scripted = |file: &str, lines| ScriptedTrack {
        samples: track_audio::read_range(&dialogue.join(file), 0, 3_600_000).unwrap(),
        lines,
    };
    let codex = Arc::new(ScriptedCodex {
        tracks: vec![
            scripted(MIC_TRACK, mic_lines),
            scripted(SYSTEM_TRACK, system_lines),
        ],
    });

    let data = work.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let (store, _) = MeetingsStore::open(data.join("meetings.db"), chrono::Utc::now());
    let store = Arc::new(store);
    let root = data.join("meetings");
    let duration_ms = track_ms_in(&dialogue) as i64;
    let mut me = Vec::new();
    for route in [OutputRoute::Speakers, OutputRoute::Headphones] {
        let id = store.insert_recording(0, MeetingTracks::Both).unwrap();
        store
            .finish_recording(id, duration_ms, duration_ms)
            .unwrap();
        store.set_output_route(id, route).unwrap();
        let dir = paths::meeting_dir(&root, id);
        std::fs::create_dir_all(&dir).unwrap();
        for file in [MIC_TRACK, SYSTEM_TRACK] {
            std::fs::copy(dialogue.join(file), dir.join(file)).unwrap();
        }
        let processor = Processor {
            store: Arc::clone(&store),
            root: root.clone(),
            transcriber: Arc::clone(&codex) as Arc<dyn Transcriber>,
            local: None,
            analyzer: Arc::new(NoModels),
            summarizer: Arc::new(CannedSummary),
            policy: RetryPolicy::default(),
            language: Some("fr".to_string()),
            events: Arc::new(NoEvents),
        };
        let operation = ProcessingOperation::new(OperationId(id as u64));
        tauri::async_runtime::block_on(processor.process(id, operation)).unwrap();
        let meeting = store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        let keys: Vec<String> = meeting
            .turns
            .iter()
            .filter(|turn| turn.speaker_key == "me")
            .flat_map(|turn| tokenize(&turn.text))
            .map(|token| token.key)
            .filter(|key| !key.is_empty())
            .collect();
        me.push(format!(" {} ", keys.join(" ")));
    }

    let rounds = said.len() / LINES.len();
    let words = |text: &str| {
        tokenize(text)
            .iter()
            .filter(|token| !token.key.is_empty())
            .count()
    };
    // The remote side's sentences of three words or more under "Me": shorter
    // ones ("Bien.") recur in other lines, so a search cannot count them.
    let echoes = |keys: &str| -> usize {
        LINES
            .iter()
            .filter(|(remote, _)| *remote)
            .flat_map(|(_, line)| sentences(line))
            .map(|sentence| join(&sentence))
            .filter(|sentence| words(sentence) >= 3)
            .map(|sentence| said_in(keys, &sentence))
            .sum()
    };
    // The user's words missing under "Me", each of their lines whole.
    let lost_words: usize = LINES
        .iter()
        .filter(|(remote, _)| !remote)
        .map(|(_, line)| rounds.saturating_sub(said_in(&me[0], line)) * words(line))
        .sum();
    let (echoes_left, echoes_on_headphones) = (echoes(&me[0]), echoes(&me[1]));
    let removed = 1.0 - echoes_left as f64 / echoes_on_headphones.max(1) as f64;
    println!(
        "Echoed sentences under Me: {echoes_on_headphones} on headphones (nothing removed), \
         {echoes_left} on speakers ({:.0} % removed). The user's words lost: {lost_words}.",
        removed * 100.0,
    );
    assert!(echoes_on_headphones > 0, "the script wrote no echo");
    assert_eq!(lost_words, 0, "the user's words were lost");
    // Six in ten when this test was written: an echo that the spread lays
    // over the user's own, louder words stays, as a sentence the user said
    // back would. Fewer means the filter got worse.
    assert!(removed >= 0.5, "only {:.0} % removed", removed * 100.0);
}

/// The length of the dialogue's microphone track.
fn track_ms_in(dialogue: &Path) -> u64 {
    let reader = hound::WavReader::open(dialogue.join(MIC_TRACK)).unwrap();
    u64::from(reader.duration()) * 1000 / u64::from(RATE)
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}
