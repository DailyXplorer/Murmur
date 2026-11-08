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
//! `a_killed_meeting_is_recovered_and_processed_at_relaunch` runs in CI, on
//! macOS and on Windows (where a kill is `TerminateProcess`), on a few
//! minutes of tones. `windows_meetings_soak` is its Windows long run: three
//! hours of tones, a fake Codex and no models, run by the nightly
//! `meetings-windows` workflow. The rest of this file is macOS-only.
//! `meetings_soak` runs three hours of two voices
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

mod children;
mod fake_codex;
mod harness;
#[cfg(target_os = "macos")]
mod mac;

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

use super::analysis::{
    platform_analyzer, AnalysisRequest, AnalyzeFuture, Analyzer, AnalyzerOutcome,
};
use super::capture::route::OutputRoute;
use super::capture::{Feeder, TrackWriter};
use super::coordinator::{
    self, CaptureBackend, Clock, Coordinator, MeetingAlert, MeetingRuntime, Notifier,
};
use super::helper;
use super::merge::{Track as Source, PLACEHOLDER};
use super::paths::{self, MIC_TRACK, SYSTEM_TRACK};
use super::processor::{MeetingProgress, ProcessEvents, Processor};
#[cfg(target_os = "macos")]
use super::store::MeetingTracks;
use super::store::{ChunkStatus, MeetingStatus, MeetingsStore, SpeakerAnalysis};
use super::summary::{CompleteFuture, Summarizer};
#[cfg(target_os = "macos")]
use super::tokens::{join, sentences, tokenize};
#[cfg(target_os = "macos")]
use super::track_audio;
use super::transcribe::{CodexTranscriber, RetryPolicy};
#[cfg(target_os = "macos")]
use super::transcribe::{TranscribeFuture, Transcriber};
use super::Library;
use crate::local_models::{manifest, Host, LocalModelStatus, LocalModels, PrepareFuture, Prepared};
use crate::settings::MeetingCaptureMode;
use crate::{OperationId, ProcessingOperation};
use children::*;
use fake_codex::*;
use harness::*;
#[cfg(target_os = "macos")]
use mac::*;

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
#[cfg(target_os = "macos")]
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

#[test]
fn a_piece_sent_again_after_its_transcription_is_counted() {
    // Pieces 1 and 2 were done, 3 and 4 were left.
    assert_eq!(sent_again(&[1, 2], &[3, 4], &[1, 2, 3, 4]), 0);
    assert_eq!(sent_again(&[1, 2], &[3, 1, 4], &[1, 2, 3, 4]), 1);
    // Two pieces of the same audio, one done before: the other is still due.
    assert_eq!(sent_again(&[7], &[7], &[7, 7]), 0);
    assert_eq!(sent_again(&[7], &[7, 7], &[7, 7]), 1);
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

#[cfg(target_os = "macos")]
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

/// The dialogue of the soak, processed as on a Mac without the on-device
/// models, with Codex writing every echo the microphone heard: the echoes
/// left under "Me" against the same meeting recorded on headphones, where
/// nothing is removed, and the user's own words lost, which must be none.
#[cfg(target_os = "macos")]
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
            vocabulary: Vec::new(),
            events: Arc::new(NoEvents),
            compress_audio: true,
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

// ---- The Windows soak ----

/// What processing may hold at its peak, as on the Mac.
#[cfg(target_os = "windows")]
const PEAK_LIMIT: u64 = 1 << 30;

/// A long meeting recorded and processed in child processes, one without a
/// stop and one killed twice (while recording, then while processing). It
/// runs the real coordinator, track writers and processor on tones, with the
/// fake Codex and no models: the speakers and the word times have their own
/// test on real voices (`analysis_cpp`). `MURMUR_SOAK_MINUTES` sets the
/// length (180 by default), `MURMUR_SOAK_REPORT` a file for the report. The
/// nightly `meetings-windows` workflow runs it:
///
/// `cargo test --locked --lib windows_meetings_soak -- --ignored --nocapture`
///
/// It never opens the app or plays a sound, and works in a temporary folder.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "three hours of tones through the real pipeline: a few minutes"]
fn windows_meetings_soak() {
    let minutes: u64 = std::env::var("MURMUR_SOAK_MINUTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(180);
    let seconds = minutes * 60;
    let work = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let dialogue = work.path().join("dialogue");
    tone_dialogue(&dialogue, 60);
    // Each answer takes 2 s, so the kill of run C finds pieces still to do.
    let codex = FakeCodex::start(Duration::from_secs(2), None);
    let fake = Some((codex.url.clone(), auth_file(work.path())));
    let child_for = |data: &Path| Child::Process {
        data: data.to_path_buf(),
        codex: fake.clone(),
        models: None,
        release_freed: false,
    };

    // Run A: recorded and processed without a stop.
    let data_a = work.path().join("a");
    let started = Instant::now();
    let a = record_in(&exe, &data_a, &dialogue, seconds, None);
    let recorded_a = started.elapsed();
    let (store_a, _) = MeetingsStore::open(data_a.join("meetings.db"), chrono::Utc::now());
    let dropped_a = store_a.dropped_ms(a.id).unwrap();
    let kept_a = track_ms(&data_a, a.id);
    let processed_a: Processed = spawn(&exe, &child_for(&data_a)).finish();
    let outcome_a = outcome(&store_a, a.id);

    // Run B: killed half way through the recording.
    let data_b = work.path().join("b");
    let b = record_in(&exe, &data_b, &dialogue, seconds, Some(seconds * 500));
    let (store_b, _) = MeetingsStore::open(data_b.join("meetings.db"), chrono::Utc::now());

    // Run C: the relaunch that processes it is killed half way through the
    // pieces, and the next one finishes.
    let child_b = child_for(&data_b);
    let done_c = process_until_killed(&exe, &child_b, &store_b, b.id, 0.5);
    let kept_b = track_ms(&data_b, b.id);
    let sent_before = codex.requests().len();
    let processed_c: Processed = spawn(&exe, &child_b).finish();
    let sent_c = codex.requests()[sent_before..].to_vec();
    let (_, done_after_c) = transcribed(&store_b, b.id).unwrap();
    let resent = sent_again(&done_c, &sent_c, &done_after_c);
    let outcome_c = outcome(&store_b, b.id);

    let checks = [
        (
            "A: each track kept every millisecond pushed",
            kept_a.iter().all(|ms| ms + MAX_LOST_MS >= a.pushed_ms),
        ),
        ("A: no audio dropped", dropped_a == 0),
        ("A is ready", outcome_a.status == MeetingStatus::Ready),
        (
            "A: the saved duration is the recorded one",
            outcome_a.duration_ms.abs_diff(a.pushed_ms as i64) < 5_000,
        ),
        (
            "A: processing holds under 1 GiB",
            processed_a.footprint < PEAK_LIMIT,
        ),
        (
            "B: each track kept all but 1 s of what was pushed",
            kept_b.iter().all(|ms| ms + MAX_LOST_MS >= b.pushed_ms),
        ),
        (
            "C: the interrupted meeting is processed at relaunch",
            processed_c.waiting.contains(&b.id),
        ),
        ("C: no transcribed piece sent again", resent == 0),
        ("C is ready", outcome_c.status == MeetingStatus::Ready),
        (
            "C: processing holds under 1 GiB",
            processed_c.footprint < PEAK_LIMIT,
        ),
    ];

    let mib = |bytes: u64| bytes / (1 << 20);
    let mut report = vec![
        "# Windows meetings soak".to_string(),
        String::new(),
        format!("- Audio: {minutes} minutes of tones, 60 s looped, on both tracks"),
        "- Codex: a local fake that tags each answer with a hash of the piece's WAV; the summary is canned; no models".to_string(),
        format!(
            "- Run A: recorded in {:.0} s, {dropped_a} ms dropped, saved as {} ms, processed in {:.1} s, peak commit {} MiB",
            recorded_a.as_secs_f64(),
            outcome_a.duration_ms,
            processed_a.seconds,
            mib(processed_a.footprint)
        ),
        format!("- Run A speakers: {:?}, bubbles {:?}", outcome_a.analysis, outcome_a.turns),
        format!(
            "- Run B: killed after {} ms pushed; the tracks held {} ms and {} ms",
            b.pushed_ms, kept_b[0], kept_b[1]
        ),
        format!(
            "- Run C: the first relaunch was killed after {} pieces; the next sent {} pieces, {resent} of them already transcribed; processed in {:.1} s, peak commit {} MiB",
            done_c.len(),
            sent_c.len(),
            processed_c.seconds,
            mib(processed_c.footprint)
        ),
        String::new(),
        "Timings as logged:".to_string(),
        String::new(),
    ];
    for (run, processed) in [("A", &processed_a), ("C", &processed_c)] {
        for logged in processed
            .log
            .iter()
            .filter(|line| line.contains("timings:"))
        {
            report.push(format!("- {run}: `{logged}`"));
        }
    }
    report.push(String::new());
    for (what, passed) in &checks {
        report.push(format!("- [{}] {what}", if *passed { "x" } else { " " }));
    }
    let report = report.join("\n");
    println!("{report}");
    if let Some(path) = std::env::var_os("MURMUR_SOAK_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    let failed: Vec<&str> = checks
        .iter()
        .filter(|(_, passed)| !passed)
        .map(|(what, _)| *what)
        .collect();
    assert!(
        failed.is_empty(),
        "failed thresholds: {}",
        failed.join("; ")
    );
}
