//! The whole local path on the real model: the download manager fetches the
//! pinned files into a temporary folder, the engine transcribes recorded
//! speech, and the model is released once idle. Needs the network and
//! spoken clips, so it runs on its own:
//!
//! `MURMUR_REAL_CLIPS=<dir> cargo test --lib engine::local::real_engine -- --ignored --nocapture`
//!
//! Each `<name>.wav` (16 kHz mono) in the folder holds speech whose exact
//! words are in `<name>.txt`. `MURMUR_REAL_MODELS` reuses a models folder
//! instead of a fresh one, and `MURMUR_REAL_REPORT` names a Markdown file
//! for the results. Intel Macs run it with `--target x86_64-apple-darwin`.

use super::session::{Runtime, Session};
use super::{LocalEngine, MAX_PIECE, SAMPLES_PER_SECOND};
use crate::local_models::{
    manifest, AnalysisSupport, Host, LocalModelState, LocalModelStatus, LocalModels, PrepareFuture,
    Prepared,
};
use crate::{OperationId, ProcessingOperation};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const IDLE_RELEASE: Duration = Duration::from_secs(5);
const MAX_WORD_ERROR_RATE: f64 = 0.15;

#[cfg(target_arch = "aarch64")]
fn helper() -> PathBuf {
    std::env::var_os("MURMUR_REAL_HELPER").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("binaries/murmur-meetings-helper-aarch64-apple-darwin")
        },
        PathBuf::from,
    )
}

/// Prints the download progress every 10 %, and loads the models once on
/// Apple silicon, as the app does.
struct TestHost {
    printed_tenth: Mutex<u64>,
}

impl Host for TestHost {
    fn notify(&self, status: LocalModelStatus) {
        if let LocalModelState::Downloading {
            done_bytes,
            total_bytes,
        } = status.state
        {
            let tenth = done_bytes * 10 / total_bytes.max(1);
            let mut printed = self.printed_tenth.lock().unwrap();
            if tenth > *printed {
                *printed = tenth;
                println!("downloaded {done_bytes} of {total_bytes} bytes");
            }
        }
    }

    fn remember_wanted(&self, _: bool) {}

    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        crate::meetings::capture::free_bytes(dir)
    }

    fn prepare(&self, dir: PathBuf, operation: ProcessingOperation) -> PrepareFuture {
        Box::pin(async move {
            #[cfg(target_arch = "aarch64")]
            {
                let args: Vec<std::ffi::OsString> =
                    vec!["prepare".into(), "--models-dir".into(), dir.into()];
                let timeout = Duration::from_secs(10 * 60);
                match crate::meetings::helper::run(&helper(), &args, &operation, timeout, |_| {})
                    .await
                {
                    Ok(crate::meetings::helper::Ended::Done) => Prepared::Done,
                    ended => {
                        println!("prepare ended: {ended:?}");
                        Prepared::Failed
                    }
                }
            }
            #[cfg(not(target_arch = "aarch64"))]
            {
                let _ = (dir, operation);
                Prepared::Done
            }
        })
    }
}

#[cfg(target_arch = "aarch64")]
fn runtime(models: &Path, scratch: &Path) -> Box<dyn Runtime> {
    Box::new(super::fluid::FluidRuntime::new(
        helper(),
        models.to_path_buf(),
        scratch.to_path_buf(),
    ))
}

#[cfg(not(target_arch = "aarch64"))]
fn runtime(models: &Path, _scratch: &Path) -> Box<dyn Runtime> {
    Box::new(super::cpu::CpuRuntime {
        model: models.join(manifest::gguf_path()),
    })
}

/// Lowercase words, punctuation and apostrophes taken as spaces.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Word edits (substitutions, insertions, deletions) per reference word.
fn word_error_rate(reference: &[String], heard: &[String]) -> f64 {
    let mut previous: Vec<usize> = (0..=heard.len()).collect();
    for (i, expected) in reference.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, word) in heard.iter().enumerate() {
            let substitution = previous[j] + usize::from(expected != word);
            row.push(substitution.min(previous[j + 1] + 1).min(row[j] + 1));
        }
        previous = row;
    }
    previous[heard.len()] as f64 / reference.len().max(1) as f64
}

/// The resident memory of the processes whose command line names `dir`, in
/// MB: the helper that holds the model on Apple silicon.
fn helper_memory_mb(dir: &Path) -> Option<u64> {
    let pids = std::process::Command::new("pgrep")
        .arg("-f")
        .arg(dir)
        .output()
        .ok()?;
    let pids = String::from_utf8_lossy(&pids.stdout);
    let pids: Vec<&str> = pids.split_whitespace().collect();
    if pids.is_empty() {
        return None;
    }
    let ps = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pids.join(",")])
        .output()
        .ok()?;
    let kilobytes: u64 = String::from_utf8_lossy(&ps.stdout)
        .split_whitespace()
        .filter_map(|value| value.parse::<u64>().ok())
        .sum();
    Some(kilobytes / 1024)
}

fn this_process_memory_mb() -> u64 {
    let ps = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&ps.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        / 1024
}

#[tokio::test]
#[ignore = "downloads the model and needs recorded speech"]
async fn the_downloaded_model_transcribes_speech_and_is_released_when_idle() {
    let clips = PathBuf::from(std::env::var("MURMUR_REAL_CLIPS").expect("MURMUR_REAL_CLIPS"));
    let fresh = tempfile::tempdir().unwrap();
    let models_dir = std::env::var_os("MURMUR_REAL_MODELS")
        .map_or_else(|| fresh.path().join("models"), PathBuf::from);
    let scratch = fresh.path().join("scratch");
    let mut report = String::new();

    let models = LocalModels::new(
        models_dir.clone(),
        manifest::this_mac(),
        AnalysisSupport::Available,
        manifest::HUGGING_FACE.to_string(),
        Arc::new(TestHost {
            printed_tenth: Mutex::new(0),
        }),
    );
    let started = Instant::now();
    models.download();
    let state = models.settled().await;
    let fetched = started.elapsed();
    assert_eq!(state, LocalModelState::Ready, "the download did not finish");
    let size_mb = models.status().size_bytes / 1_000_000;
    writeln!(
        report,
        "Model: {} file(s) from {} pinned revision(s), {size_mb} MB, SHA-256 checked, \
         downloaded and prepared in {:.0} s.\n",
        manifest::this_mac()
            .iter()
            .map(|source| source.files.len())
            .sum::<usize>(),
        manifest::this_mac().len(),
        fetched.as_secs_f64()
    )
    .unwrap();

    let engine = LocalEngine {
        session: Session::with_idle_release(
            runtime(&models_dir, &scratch),
            Box::new(|| false),
            IDLE_RELEASE,
        ),
        max_piece: MAX_PIECE,
    };
    let started = Instant::now();
    engine.session.warm().await.unwrap();
    writeln!(
        report,
        "Model load: {:.2} s.\n",
        started.elapsed().as_secs_f64()
    )
    .unwrap();

    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&clips)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .collect();
    wavs.sort();
    assert!(!wavs.is_empty(), "no clips in {}", clips.display());
    report.push_str("| Clip | Audio | Pieces | Latency | WER | Expected | Heard |\n");
    report.push_str("|---|---|---|---|---|---|---|\n");
    let operation = ProcessingOperation::new(OperationId(1));
    let mut too_wrong = Vec::new();
    for wav in &wavs {
        let samples = crate::audio_toolkit::read_wav_samples(wav).unwrap();
        let expected = std::fs::read_to_string(wav.with_extension("txt")).unwrap();
        let pieces = crate::audio_toolkit::pauses::split_at_pauses(&samples, MAX_PIECE).len();
        let started = Instant::now();
        let heard = engine.transcribe(&samples, &[], &operation).await.unwrap();
        let latency = started.elapsed();
        let rate = word_error_rate(&words(&expected), &words(&heard));
        let name = wav.file_stem().unwrap().to_string_lossy();
        println!("{name}: {rate:.3} in {latency:?}: {heard}");
        writeln!(
            report,
            "| {name} | {:.1} s | {pieces} | {:.2} s | {:.1} % | {} | {} |",
            samples.len() as f64 / SAMPLES_PER_SECOND as f64,
            latency.as_secs_f64(),
            rate * 100.0,
            expected.trim(),
            heard.trim()
        )
        .unwrap();
        if rate >= MAX_WORD_ERROR_RATE {
            too_wrong.push(format!("{name}: {:.1} %", rate * 100.0));
        }
    }

    let loaded_mb = helper_memory_mb(&models_dir);
    let process_loaded_mb = this_process_memory_mb();
    tokio::time::sleep(IDLE_RELEASE + Duration::from_secs(3)).await;
    let released = !engine.session.is_loaded().await;
    let after_mb = helper_memory_mb(&models_dir);
    writeln!(
        report,
        "\nIdle release ({IDLE_RELEASE:?} injected): model unloaded = {released}. \
         Helper memory while loaded: {}; after release: {}. Test process: {process_loaded_mb} MB \
         loaded, {} MB after release.",
        loaded_mb.map_or("no helper process".to_string(), |mb| format!("{mb} MB")),
        after_mb.map_or("no helper process".to_string(), |mb| format!("{mb} MB")),
        this_process_memory_mb()
    )
    .unwrap();
    if let Some(path) = std::env::var_os("MURMUR_REAL_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    println!("{report}");
    assert!(
        too_wrong.is_empty(),
        "word error rate too high: {too_wrong:?}"
    );
    assert!(
        released,
        "the model was not released after {IDLE_RELEASE:?}"
    );
    if cfg!(target_arch = "aarch64") {
        assert!(loaded_mb.is_some(), "no helper held the model");
        assert_eq!(after_mb, None, "the helper outlived the release");
    }
}

#[test]
fn the_word_error_rate_counts_each_edit_once() {
    let reference = words("Bonjour, l'équipe de Marseille.");
    assert_eq!(
        word_error_rate(&reference, &words("bonjour l’équipe de marseille")),
        0.0
    );
    assert_eq!(
        word_error_rate(&reference, &words("bonjour équipe de marseille")),
        0.2
    );
    assert_eq!(
        word_error_rate(&reference, &words("bonsoir l équipe à marseille")),
        0.4
    );
}
