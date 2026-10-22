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

use super::boost::BoostTerms;
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
        let heard = engine
            .transcribe(&samples, &BoostTerms::default(), &operation)
            .await
            .unwrap();
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

#[cfg(target_arch = "aarch64")]
fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(target_arch = "aarch64")]
fn median_and_max(mut values: Vec<Duration>) -> (Duration, Duration) {
    values.sort();
    (values[values.len() / 2], values[values.len() - 1])
}

#[cfg(target_arch = "aarch64")]
const BOOST_RUNS: usize = 5;

/// Custom-word boosting on the real model, Apple silicon only:
///
/// `MURMUR_REAL_BOOST_CLIPS=<dir> MURMUR_REAL_MODELS=<dir> cargo test --lib
/// engine::local::real_engine::boosting -- --ignored --nocapture`
///
/// The folder holds `target-*.wav`, speech with custom words, `name-*.wav`,
/// speech with a name that is only in the window, `control-*.wav`, speech
/// with neither, each with its exact text in `<name>.txt`, and the custom
/// words and window names in `custom.txt` and `names.txt`, one per line.
/// The words of `name-only.txt` are custom words for the other clips and
/// window names for the `name-*` clips. The models folder gets the optional
/// CTC model as the app does. Each clip runs on Parakeet alone, then with the
/// CTC model and no terms, the custom words, and the custom words with the
/// window names. A word of a control clip that the terms change is a false
/// replacement.
#[cfg(target_arch = "aarch64")]
#[tokio::test]
#[ignore = "downloads the models and needs recorded speech"]
async fn boosting_hears_custom_words_and_leaves_other_speech_alone() {
    use super::boost::boost_terms;

    let clips = PathBuf::from(std::env::var("MURMUR_REAL_BOOST_CLIPS").expect("clips"));
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
    models.resume_on_launch(true);
    assert_eq!(models.settled().await, LocalModelState::Ready);
    models.fetched().await;
    let ctc = manifest::this_mac()
        .iter()
        .find(|source| source.optional)
        .unwrap();
    assert!(
        models_dir.join(ctc.folder).join("vocab.json").exists(),
        "the CTC model was not fetched"
    );

    // Parakeet without the CTC model next to it, so the helper boosts nothing.
    let plain_dir = fresh.path().join("plain");
    std::fs::create_dir_all(&plain_dir).unwrap();
    let asr = manifest::this_mac()[0].folder;
    std::os::unix::fs::symlink(models_dir.join(asr), plain_dir.join(asr)).unwrap();

    let (custom, names) = (
        lines(&clips.join("custom.txt")),
        lines(&clips.join("names.txt")),
    );
    let name_only = lines(&clips.join("name-only.txt"));
    // The custom words and window names a clip is dictated with.
    let lists = |clip: &str| -> (Vec<String>, Vec<String>) {
        if !clip.starts_with("name-") {
            return (custom.clone(), names.clone());
        }
        let custom = custom
            .iter()
            .filter(|word| !name_only.contains(word))
            .cloned()
            .collect();
        (custom, [name_only.clone(), names.clone()].concat())
    };
    for clip in ["target", "name-"] {
        let (custom, names) = lists(clip);
        let terms = boost_terms(&custom, &names);
        writeln!(
            report,
            "{clip} clips: custom words ({}): {}. Window names ({}, capped at {} terms in \
             all): {}.\n",
            terms.words.len(),
            terms.words.join(", "),
            terms.names.len(),
            super::boost::MAX_BOOST_TERMS,
            terms.names.join(", ")
        )
        .unwrap();
    }

    let engine = |dir: &Path| LocalEngine {
        session: Session::with_idle_release(
            runtime(dir, &scratch),
            Box::new(|| false),
            Duration::from_secs(30 * 60),
        ),
        max_piece: MAX_PIECE,
    };
    let (plain, boosting) = (engine(&plain_dir), engine(&models_dir));
    for (label, engine) in [("Parakeet alone", &plain), ("with CTC", &boosting)] {
        let started = Instant::now();
        engine.session.warm().await.unwrap();
        writeln!(
            report,
            "Load {label}: {:.2} s.",
            started.elapsed().as_secs_f64()
        )
        .unwrap();
    }
    writeln!(
        report,
        "\nHelper resident memory: Parakeet alone {} MB, with the CTC model {} MB.\n",
        helper_memory_mb(&plain_dir).unwrap_or(0),
        helper_memory_mb(&models_dir).unwrap_or(0)
    )
    .unwrap();

    let modes: [(&str, &LocalEngine); 4] = [
        ("Parakeet alone", &plain),
        ("CTC, no terms", &boosting),
        ("custom words", &boosting),
        ("with window names", &boosting),
    ];
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&clips)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .collect();
    wavs.sort();
    assert!(!wavs.is_empty(), "no clips in {}", clips.display());

    let operation = ProcessingOperation::new(OperationId(1));
    let mut texts =
        String::from("| Clip | Expected | Mode | Heard | WER |\n|---|---|---|---|---|\n");
    let mut timings = format!(
        "| Clip | Mode | First run | Median of the next {BOOST_RUNS} | Max |\n|---|---|---|---|---|\n"
    );
    let mut medians = vec![Vec::new(); modes.len()];
    let mut false_replacements = Vec::new();
    let (mut fixed, mut worse) = (Vec::new(), Vec::new());
    let (mut fixed_by_names, mut worse_by_names) = (Vec::new(), Vec::new());
    for wav in &wavs {
        let name = wav.file_stem().unwrap().to_string_lossy().to_string();
        let samples = crate::audio_toolkit::read_wav_samples(wav).unwrap();
        let expected = std::fs::read_to_string(wav.with_extension("txt")).unwrap();
        let (custom, names) = lists(&name);
        let terms_by_mode = [
            BoostTerms::default(),
            BoostTerms::default(),
            boost_terms(&custom, &[]),
            boost_terms(&custom, &names),
        ];
        let mut heard_by_mode = Vec::new();
        for (index, ((mode, engine), terms)) in modes.iter().zip(&terms_by_mode).enumerate() {
            // The terms changed since the last run: the helper builds its
            // rescorer again.
            let started = Instant::now();
            engine
                .transcribe(&samples, terms, &operation)
                .await
                .unwrap();
            let first = started.elapsed();
            let mut runs = Vec::new();
            let mut heard = String::new();
            for _ in 0..BOOST_RUNS {
                let started = Instant::now();
                heard = engine
                    .transcribe(&samples, terms, &operation)
                    .await
                    .unwrap();
                runs.push(started.elapsed());
            }
            let (median, max) = median_and_max(runs);
            medians[index].push(median);
            let rate = word_error_rate(&words(&expected), &words(&heard));
            println!("{name} / {mode}: {rate:.3} in {median:?}: {heard}");
            writeln!(
                texts,
                "| {name} | {} | {mode} | {} | {:.1} % |",
                expected.trim(),
                heard.trim(),
                rate * 100.0
            )
            .unwrap();
            writeln!(
                timings,
                "| {name} | {mode} | {:.0} ms | {:.0} ms | {:.0} ms |",
                first.as_secs_f64() * 1_000.0,
                median.as_secs_f64() * 1_000.0,
                max.as_secs_f64() * 1_000.0
            )
            .unwrap();
            heard_by_mode.push(words(&heard));
        }
        if name.starts_with("control") {
            for (mode, heard) in modes.iter().zip(&heard_by_mode).skip(2) {
                let changed =
                    word_error_rate(&heard_by_mode[1], heard) * heard_by_mode[1].len() as f64;
                if changed > 0.0 {
                    false_replacements.push(format!("{name} ({}): {changed:.0} word(s)", mode.0));
                }
            }
        }
        let rate = |heard: &[String]| word_error_rate(&words(&expected), heard);
        let (no_terms, custom_words, with_names) = (
            rate(&heard_by_mode[1]),
            rate(&heard_by_mode[2]),
            rate(&heard_by_mode[3]),
        );
        if custom_words < no_terms {
            fixed.push(name.clone());
        } else if custom_words > no_terms {
            worse.push(name.clone());
        }
        if with_names < custom_words {
            fixed_by_names.push(name.clone());
        } else if with_names > custom_words {
            worse_by_names.push(name.clone());
        }
    }
    report.push_str(&texts);
    report.push('\n');
    report.push_str(&timings);
    report.push_str("\nMedian over the clips of each clip's median:\n\n");
    for ((mode, ..), mut values) in modes.iter().zip(medians) {
        values.sort();
        writeln!(
            report,
            "- {mode}: {:.0} ms",
            values[values.len() / 2].as_secs_f64() * 1_000.0
        )
        .unwrap();
    }
    writeln!(
        report,
        "\nFalse replacements in the control clips: {}.",
        if false_replacements.is_empty() {
            "none".to_string()
        } else {
            false_replacements.join("; ")
        }
    )
    .unwrap();
    writeln!(
        report,
        "\nWith the custom words, {} clip(s) heard better ({}), {} worse ({}).",
        fixed.len(),
        fixed.join(", "),
        worse.len(),
        worse.join(", ")
    )
    .unwrap();
    writeln!(
        report,
        "\nAdding the window names, {} clip(s) heard better than with the custom words \
         alone ({}), {} worse ({}).",
        fixed_by_names.len(),
        fixed_by_names.join(", "),
        worse_by_names.len(),
        worse_by_names.join(", ")
    )
    .unwrap();
    if let Some(path) = std::env::var_os("MURMUR_REAL_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    println!("{report}");
    assert!(!fixed.is_empty(), "the custom words fixed no clip");
    assert!(worse.is_empty(), "the custom words made {worse:?} worse");
    assert!(
        false_replacements.is_empty(),
        "the terms replaced words of a control clip: {false_replacements:?}"
    );
    assert!(
        fixed_by_names.iter().any(|clip| clip.starts_with("name-")),
        "the window names fixed no name-only clip"
    );
    assert!(
        worse_by_names.is_empty(),
        "the window names made {worse_by_names:?} worse"
    );
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
