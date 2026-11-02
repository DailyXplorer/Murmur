use super::real_engine::{helper_memory_mb, runtime, word_error_rate, words};
use super::session::{Background, Session};
use super::{padded, speech_pieces, MAX_PIECE};
use crate::meetings::live::chunker::{Chunk, Chunker, Cut, LiveBlock, RATE};
use crate::meetings::live::seam::{heard, Draft};
use crate::meetings::merge::Track as Source;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(60);
const DICTATIONS: usize = 7;

fn read(path: &Path) -> Vec<f32> {
    let reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().sample_rate, RATE as u32, "{}", path.display());
    reader
        .into_samples::<i16>()
        .map(|sample| f32::from(sample.unwrap()) / 32_768.0)
        .collect()
}

fn helper_cpu(models: &Path) -> Duration {
    let pids = std::process::Command::new("pgrep")
        .arg("-f")
        .arg(models)
        .output()
        .unwrap();
    let pids = String::from_utf8_lossy(&pids.stdout);
    let pids: Vec<&str> = pids.split_whitespace().collect();
    let ps = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pids.join(",")])
        .output()
        .unwrap();
    String::from_utf8_lossy(&ps.stdout)
        .split_whitespace()
        .map(|time| {
            let seconds = time.split(':').fold(0.0, |total, part| {
                total * 60.0 + part.parse::<f64>().unwrap_or(0.0)
            });
            Duration::from_secs_f64(seconds)
        })
        .sum()
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

fn ms(samples: u64) -> u64 {
    samples * 1_000 / RATE as u64
}

async fn live_pass(session: &Arc<Session>, audio: &[f32]) -> (String, Vec<f64>, Vec<f64>, usize) {
    let mut chunker = Chunker::new(Source::Mic);
    let mut chunks: Vec<Chunk> = Vec::new();
    for (index, block) in audio.chunks(170).enumerate() {
        chunks.extend(chunker.push(LiveBlock {
            start_sample: (index * 170) as u64,
            samples: block,
        }));
    }
    chunks.extend(chunker.finish());
    let mut draft = Draft::new(Source::Mic);
    let mut text = Vec::new();
    let mut lags = Vec::new();
    let mut first_word_lags = Vec::new();
    let mut free = 0.0f64;
    let mut sent = 0;
    for chunk in &chunks {
        let ready = chunk.end_sample() as f64 / RATE as f64;
        let mut words_heard = Vec::new();
        let mut done = ready;
        if !chunk.silent() {
            sent += 1;
            let started = Instant::now();
            let result = session
                .transcribe_background(&padded(&chunk.samples), LIMIT)
                .await;
            let decode = started.elapsed().as_secs_f64();
            match result {
                Background::Done(words) => words_heard = heard(&words, ms(chunk.start_sample)),
                Background::Busy => panic!("busy without a dictation"),
                Background::Failed(error) => panic!("{error:#}"),
            }
            done = ready.max(free) + decode;
            free = done;
        }
        draft.accept(
            ms(chunk.start_sample),
            ms(chunk.overlap as u64),
            words_heard,
            ms(chunk.settled_sample()),
            chunk.cut == Cut::Hard,
        );
        for entry in draft.publish() {
            lags.push(done - entry.end_ms as f64 / 1_000.0);
            first_word_lags.push(done - entry.start_ms as f64 / 1_000.0);
            text.push(entry.text);
        }
    }
    let end = (audio.len() as f64 / RATE as f64).max(free);
    for entry in draft.finish() {
        lags.push(end - entry.end_ms as f64 / 1_000.0);
        first_word_lags.push(end - entry.start_ms as f64 / 1_000.0);
        text.push(entry.text);
    }
    (text.join(" "), lags, first_word_lags, sent)
}

async fn dictation(session: &Arc<Session>, clip: &[f32]) -> Duration {
    let started = Instant::now();
    let _dictation = session.dictation();
    for range in speech_pieces(clip, MAX_PIECE) {
        session
            .transcribe(&padded(&clip[range]), &[], None, LIMIT)
            .await
            .unwrap();
    }
    started.elapsed()
}

fn median(mut values: Vec<Duration>) -> (Duration, Duration) {
    values.sort();
    (values[values.len() / 2], values[values.len() - 1])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the models and recorded speech: MURMUR_REAL_LIVE=<wav>,<wav> (16 kHz mono) MURMUR_REAL_LIVE_REFS=<dir of <name>.txt> MURMUR_REAL_MODELS=<dir>, run with --ignored --nocapture"]
async fn live_replay() {
    let clips = std::env::var("MURMUR_REAL_LIVE").expect("MURMUR_REAL_LIVE");
    let refs =
        PathBuf::from(std::env::var("MURMUR_REAL_LIVE_REFS").expect("MURMUR_REAL_LIVE_REFS"));
    let models = PathBuf::from(std::env::var("MURMUR_REAL_MODELS").expect("MURMUR_REAL_MODELS"));
    let scratch = tempfile::tempdir().unwrap();
    let session = Session::new(runtime(&models, scratch.path()), Box::new(|| false));
    session.warm().await.unwrap();
    let mut report = String::from(
        "| clip | pieces | lag behind sentence end p50 / p95 s | lag behind sentence start p50 / p95 s | helper CPU % of a core | helper RSS MB before → after | WER live | WER after Stop (local) | WER live vs after Stop |\n|---|---|---|---|---|---|---|---|---|\n",
    );
    for clip in clips.split(',') {
        let path = PathBuf::from(clip);
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let audio = read(&path);
        let seconds = audio.len() as f64 / RATE as f64;
        let reference = words(&std::fs::read_to_string(refs.join(format!("{name}.txt"))).unwrap());

        let memory_before = helper_memory_mb(&models).unwrap_or(0);
        let cpu_before = helper_cpu(&models);
        let (live, mut lags, mut first_word_lags, sent) = live_pass(&session, &audio).await;
        let cpu = helper_cpu(&models).saturating_sub(cpu_before);
        let memory_after = helper_memory_mb(&models).unwrap_or(0);

        let mut after_stop = Vec::new();
        for range in speech_pieces(&audio, MAX_PIECE) {
            after_stop.push(
                session
                    .transcribe(&padded(&audio[range]), &[], None, LIMIT)
                    .await
                    .unwrap()
                    .text,
            );
        }
        let after_stop = words(&after_stop.join(" "));
        let live_words = words(&live);
        writeln!(
            report,
            "| {name} | {sent} | {:.1} / {:.1} | {:.1} / {:.1} | {:.2} | {memory_before} → {memory_after} | {:.3} | {:.3} | {:.3} |",
            percentile(&mut lags, 0.5),
            percentile(&mut lags, 0.95),
            percentile(&mut first_word_lags, 0.5),
            percentile(&mut first_word_lags, 0.95),
            100.0 * cpu.as_secs_f64() / seconds,
            word_error_rate(&reference, &live_words),
            word_error_rate(&reference, &after_stop),
            word_error_rate(&after_stop, &live_words),
        )
        .unwrap();
    }

    let first = read(&PathBuf::from(clips.split(',').next().unwrap()));
    let clip = first[..5 * RATE].to_vec();
    let piece = first[5 * RATE..15 * RATE].to_vec();
    let mut idle = Vec::new();
    for _ in 0..DICTATIONS {
        idle.push(dictation(&session, &clip).await);
    }
    let running = Arc::new(AtomicBool::new(true));
    let live = tokio::spawn({
        let session = Arc::clone(&session);
        let running = Arc::clone(&running);
        async move {
            let mut done = 0;
            while running.load(Ordering::SeqCst) {
                match session.transcribe_background(&piece, LIMIT).await {
                    Background::Done(_) => done += 1,
                    Background::Busy => tokio::time::sleep(Duration::from_millis(250)).await,
                    Background::Failed(error) => panic!("{error:#}"),
                }
            }
            done
        }
    });
    let mut busy = Vec::new();
    for index in 0..DICTATIONS {
        tokio::time::sleep(Duration::from_millis(400 + 37 * index as u64)).await;
        busy.push(dictation(&session, &clip).await);
    }
    running.store(false, Ordering::SeqCst);
    let pieces = live.await.unwrap();
    let (idle_median, idle_max) = median(idle);
    let (busy_median, busy_max) = median(busy);
    writeln!(
        report,
        "\n5 s dictation, {DICTATIONS} runs: idle median {} ms (max {} ms); while live 10 s pieces run back to back ({pieces} done) median {} ms (max {} ms)",
        idle_median.as_millis(),
        idle_max.as_millis(),
        busy_median.as_millis(),
        busy_max.as_millis(),
    )
    .unwrap();
    if let Some(path) = std::env::var_os("MURMUR_REAL_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    println!("{report}");
}
