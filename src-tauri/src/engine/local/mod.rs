//! Parakeet Ultra on this Mac. Apple silicon runs the Core ML models
//! in the meetings helper (`fluid`), other Macs the GGUF model on the CPU in
//! the app (`cpu`). Both get pieces short enough for one pass, cut at pauses.

pub mod boost;
#[cfg(target_arch = "x86_64")]
mod cpu;
#[cfg(target_arch = "aarch64")]
mod fluid;
mod holes;
#[cfg(all(test, target_arch = "aarch64"))]
mod live_replay;
#[cfg(all(test, target_os = "macos"))]
mod real_engine;
pub mod session;
pub mod unknown;

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::gain::FRAME;
use crate::audio_toolkit::pauses::{speech_frames, speech_pieces};
use crate::local_models::LocalModels;
use crate::ProcessingOperation;
use session::{Heard, Runtime, Session};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const SAMPLES_PER_SECOND: usize = TARGET_SAMPLE_RATE as usize;

/// FluidAudio's encoder window is 15 s; above it FluidAudio cuts the audio
/// itself, with overlapping seams.
#[cfg(target_arch = "aarch64")]
const MAX_PIECE: usize = SAMPLES_PER_SECOND * 29 / 2;
/// transcribe.cpp turns to local attention above 20.5 s, less accurate.
#[cfg(not(target_arch = "aarch64"))]
const MAX_PIECE: usize = SAMPLES_PER_SECOND * 20;

/// FluidAudio refuses audio under 0.3 s; shorter speech gets silence after it.
const MIN_PIECE: usize = SAMPLES_PER_SECOND / 2;

/// A run may take this long plus the length of its audio.
const RUN_ALLOWANCE: Duration = Duration::from_secs(30);

/// In the app's cache: the pieces the helper reads, each deleted once
/// answered. A quit or a crash during a request leaves one behind.
const SCRATCH_DIR: &str = "local-stt";

pub struct LocalEngine {
    session: Arc<Session>,
    max_piece: usize,
}

impl LocalEngine {
    fn new(
        runtime: Box<dyn Runtime>,
        max_piece: usize,
        recording: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            session: Session::new(runtime, recording),
            max_piece,
        }
    }

    /// Loads the model in the background, so it is ready when the user
    /// stops speaking.
    pub fn warm(&self) {
        let session = Arc::clone(&self.session);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = session.warm().await {
                log::warn!("Cannot load the local model: {error:#}");
            }
        });
    }

    /// Releases the model, once the dictation running, if any, is done. The
    /// CPU model maps its file, so deleted models keep their disk space
    /// until then.
    // Windows deletes through `release_then`; its tests still call this.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub async fn release(&self) {
        self.session.release().await;
    }

    /// Releases the model, once the dictation running, if any, is done, and
    /// loads none until `then` is done. Windows may refuse to delete or
    /// replace the file of a loaded model.
    #[cfg(target_os = "windows")]
    pub async fn release_then<T>(&self, then: impl std::future::Future<Output = T>) -> T {
        self.session.release_then(then).await
    }

    /// Transcribes 16 kHz mono audio of any length, listening for `terms`
    /// (see `boost`) and in `language` (a code such as `fr`) when it is
    /// known, else detecting it. Silence gives no text and never reaches the
    /// model, which would invent words.
    pub async fn transcribe(
        &self,
        samples: &[f32],
        terms: &[String],
        language: Option<&str>,
        operation: &ProcessingOperation,
    ) -> anyhow::Result<String> {
        #[cfg(debug_assertions)]
        if std::env::var("MURMUR_FORCE_LOCAL_FAILURE").is_ok() {
            anyhow::bail!("Simulated local transcription failure (MURMUR_FORCE_LOCAL_FAILURE)");
        }
        // A release asked between two pieces waits for the last one.
        let _dictation = self.session.dictation();
        let speech = speech_frames(samples);
        let mut texts = Vec::new();
        for range in speech_pieces(samples, self.max_piece) {
            let frames = range.start / FRAME..range.end.div_ceil(FRAME);
            let piece = &samples[range];
            texts.push(
                self.piece(piece, &speech[frames], terms, language, operation)
                    .await?,
            );
        }
        Ok(join(texts))
    }

    /// One piece, and again each stretch of it the model left without a
    /// word (see `holes`). A stretch that fails the second time keeps the
    /// piece's first text.
    async fn piece(
        &self,
        piece: &[f32],
        speech: &[bool],
        terms: &[String],
        language: Option<&str>,
        operation: &ProcessingOperation,
    ) -> anyhow::Result<String> {
        let heard = self.hear(piece, terms, language, operation).await?;
        let mut fills = Vec::new();
        for hole in holes::find(&heard, piece.len(), speech) {
            match self
                .hear(&piece[hole.audio], terms, language, operation)
                .await
            {
                Ok(again) => fills.push((hole.words_before, again.text)),
                Err(error) if operation.is_cancelled() => return Err(error),
                Err(error) => log::warn!("Cannot transcribe a stretch again: {error:#}"),
            }
        }
        if !fills.is_empty() {
            log::info!("Local piece: {} stretch(es) transcribed again", fills.len());
        }
        Ok(holes::fill(&heard.text, &fills))
    }

    /// One request to the model, ended by a cancelled operation.
    async fn hear(
        &self,
        audio: &[f32],
        terms: &[String],
        language: Option<&str>,
        operation: &ProcessingOperation,
    ) -> anyhow::Result<Heard> {
        let audio = padded(audio);
        let limit = RUN_ALLOWANCE + seconds(audio.len());
        tokio::select! {
            biased;
            () = operation.cancelled() => anyhow::bail!("Local transcription cancelled"),
            heard = self.session.transcribe(&audio, terms, language, limit) => heard,
        }
    }

    /// One piece of a live meeting, at most one pass long, with its words'
    /// times. Dictation goes first: see `Session::transcribe_background`.
    pub async fn transcribe_live(&self, samples: &[f32]) -> session::Background {
        let piece = padded(samples);
        let limit = RUN_ALLOWANCE + seconds(piece.len());
        self.session.transcribe_background(&piece, limit).await
    }

    /// Keeps the model loaded through a live meeting's silences.
    pub fn keep_loaded(&self) {
        self.session.keep_loaded();
    }
}

fn seconds(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / SAMPLES_PER_SECOND as f64)
}

fn padded(piece: &[f32]) -> Vec<f32> {
    let mut piece = piece.to_vec();
    if piece.len() < MIN_PIECE {
        piece.resize(MIN_PIECE, 0.0);
    }
    piece
}

fn join(texts: Vec<String>) -> String {
    texts
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_arch = "aarch64")]
fn runtime(app: &AppHandle, models: &LocalModels) -> Option<Box<dyn Runtime>> {
    let program = crate::meetings::helper::helper_path()?;
    let scratch = app.path().app_cache_dir().ok()?.join(SCRATCH_DIR);
    Some(Box::new(fluid::FluidRuntime::new(
        program,
        models.dir().to_path_buf(),
        scratch,
        models.subscribe(),
    )))
}

#[cfg(not(target_arch = "aarch64"))]
fn runtime(_app: &AppHandle, models: &LocalModels) -> Option<Box<dyn Runtime>> {
    Some(Box::new(cpu::CpuRuntime {
        models: models.dir().to_path_buf(),
    }))
}

/// An engine with its own model, loaded on first use. `recording` tells
/// whether a dictation is being recorded, which keeps the model loaded.
pub fn engine(
    app: &AppHandle,
    recording: Box<dyn Fn() -> bool + Send + Sync>,
) -> Option<LocalEngine> {
    let models = app.try_state::<Arc<LocalModels>>()?;
    Some(LocalEngine::new(
        runtime(app, &models)?,
        MAX_PIECE,
        recording,
    ))
}

/// Registers the dictation engine, after the local models. Pieces an
/// earlier run left go first, before any engine writes one.
pub fn init(app: &AppHandle) {
    if let Ok(cache) = app.path().app_cache_dir() {
        remove_scratch(&cache);
    }
    let handle = app.clone();
    let recording = Box::new(move || {
        handle
            .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
            .is_some_and(|recorder| recorder.is_recording())
    });
    match engine(app, recording) {
        Some(engine) => {
            app.manage(Arc::new(engine));
        }
        None => log::warn!("Local transcription is unavailable in this build"),
    }
}

/// Deletes the pieces a quit or a crash left in `cache`.
fn remove_scratch(cache: &std::path::Path) {
    match std::fs::remove_dir_all(cache.join(SCRATCH_DIR)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("Cannot empty the local transcription pieces: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::session::{BoxFuture, Loaded};
    use super::*;
    use crate::OperationId;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Mutex;

    #[test]
    fn pieces_left_by_an_earlier_run_are_deleted() {
        let cache = tempfile::tempdir().unwrap();
        let scratch = cache.path().join(SCRATCH_DIR);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("piece-1.wav"), [0u8; 44]).unwrap();

        remove_scratch(cache.path());

        assert!(!scratch.exists());
        // Nothing to delete is not an error.
        remove_scratch(cache.path());
    }

    /// Answers each piece with its length in samples, or the given text,
    /// and keeps the terms each piece was given.
    struct Lengths {
        pieces: Arc<Mutex<Vec<usize>>>,
        terms: Arc<Mutex<Vec<Vec<String>>>>,
        answer: Option<&'static str>,
    }

    impl Runtime for Lengths {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            let model = Lengths {
                pieces: Arc::clone(&self.pieces),
                terms: Arc::clone(&self.terms),
                answer: self.answer,
            };
            Box::pin(async move { Ok(Box::new(model) as Box<dyn Loaded>) })
        }
    }

    impl Loaded for Lengths {
        fn transcribe<'a>(
            &'a mut self,
            samples: &'a [f32],
            terms: &'a [String],
            _: Option<&'a str>,
        ) -> BoxFuture<'a, anyhow::Result<Heard>> {
            self.pieces.lock().unwrap().push(samples.len());
            self.terms.lock().unwrap().push(terms.to_vec());
            let text = self
                .answer
                .map_or_else(|| format!(" {} ", samples.len()), str::to_string);
            Box::pin(async move {
                Ok(Heard {
                    text,
                    words: Vec::new(),
                })
            })
        }
    }

    type Seen<T> = Arc<Mutex<Vec<T>>>;

    fn engine(answer: Option<&'static str>) -> (LocalEngine, Seen<usize>) {
        let (engine, pieces, _) = engine_with_terms(answer);
        (engine, pieces)
    }

    fn engine_with_terms(
        answer: Option<&'static str>,
    ) -> (LocalEngine, Seen<usize>, Seen<Vec<String>>) {
        let pieces = Arc::new(Mutex::new(Vec::new()));
        let terms = Arc::new(Mutex::new(Vec::new()));
        let runtime = Lengths {
            pieces: Arc::clone(&pieces),
            terms: Arc::clone(&terms),
            answer,
        };
        let engine = LocalEngine::new(
            Box::new(runtime),
            10 * SAMPLES_PER_SECOND,
            Box::new(|| false),
        );
        (engine, pieces, terms)
    }

    /// Speech at syllable rhythm (40 ms loud, 20 ms 10 dB softer), `seconds`
    /// long, with `gap` seconds of silence in the middle.
    fn speech(seconds: f32, gap: f32) -> Vec<f32> {
        let total = (seconds * SAMPLES_PER_SECOND as f32) as usize;
        let gap_len = (gap * SAMPLES_PER_SECOND as f32) as usize;
        let gap_start = (total - gap_len) / 2;
        let syllable = SAMPLES_PER_SECOND * 60 / 1_000;
        (0..total)
            .map(|i| {
                if (gap_start..gap_start + gap_len).contains(&i) {
                    return 0.0;
                }
                let level = if i % syllable < syllable * 2 / 3 {
                    0.1
                } else {
                    0.0316
                };
                level * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin()
            })
            .collect()
    }

    fn operation() -> ProcessingOperation {
        ProcessingOperation::new(OperationId(1))
    }

    #[tokio::test]
    async fn a_long_dictation_is_cut_at_its_pause_and_the_texts_joined() {
        let (engine, pieces) = engine(None);
        let audio = speech(16.0, 1.0);
        let text = engine
            .transcribe(&audio, &[], None, &operation())
            .await
            .unwrap();
        let pieces = pieces.lock().unwrap().clone();
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert_eq!(pieces.iter().sum::<usize>(), audio.len());
        let cut = pieces[0] as f32 / SAMPLES_PER_SECOND as f32;
        assert!(
            (7.5..8.5).contains(&cut),
            "cut at {cut} s, not in the pause"
        );
        assert_eq!(text, format!("{} {}", pieces[0], pieces[1]));
    }

    #[tokio::test]
    async fn a_short_dictation_is_one_piece_and_a_very_short_one_is_padded() {
        let (engine, pieces) = engine(None);
        engine
            .transcribe(&speech(4.0, 0.0), &[], None, &operation())
            .await
            .unwrap();
        engine
            .transcribe(&speech(0.25, 0.0), &[], None, &operation())
            .await
            .unwrap();
        assert_eq!(*pieces.lock().unwrap(), [4 * SAMPLES_PER_SECOND, MIN_PIECE]);
    }

    #[tokio::test]
    async fn silence_never_reaches_the_model() {
        let (engine, pieces) = engine(Some("Thank you."));
        let text = engine
            .transcribe(&vec![0.0; 5 * SAMPLES_PER_SECOND], &[], None, &operation())
            .await
            .unwrap();
        assert_eq!(text, "");
        assert!(pieces.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_pause_a_dictation_ends_on_never_reaches_the_model() {
        let (engine, pieces) = engine(Some("Thank you."));
        // Longer than one piece: the cut lands in the closing pause, and
        // what follows it is room noise only.
        let mut audio = speech(9.0, 0.0);
        audio.extend(
            (0..3 * SAMPLES_PER_SECOND)
                .map(|i| 0.001 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin()),
        );
        let text = engine
            .transcribe(&audio, &[], None, &operation())
            .await
            .unwrap();
        let pieces = pieces.lock().unwrap().clone();
        assert_eq!(pieces.len(), 1, "{pieces:?}");
        assert!(pieces[0] < audio.len());
        assert_eq!(text, "Thank you.");
    }

    #[tokio::test]
    async fn a_cancelled_dictation_stops() {
        let (engine, _) = engine(None);
        let cancelled = operation();
        cancelled.cancel();
        assert!(engine
            .transcribe(&speech(4.0, 0.0), &[], None, &cancelled)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn every_piece_of_a_dictation_listens_for_its_terms() {
        let (engine, _, seen) = engine_with_terms(None);
        let terms = vec!["Siobhán".to_string(), "Kubernetes".to_string()];
        engine
            .transcribe(&speech(16.0, 1.0), &terms, None, &operation())
            .await
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), [terms.clone(), terms]);
    }

    type Answer = Arc<dyn Fn(usize) -> anyhow::Result<Heard> + Send + Sync>;

    /// Answers each request from its length in samples, and keeps the
    /// length and language of every request.
    struct Scripted {
        answer: Answer,
        seen: Seen<(usize, Option<String>)>,
    }

    impl Runtime for Scripted {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            let model = Scripted {
                answer: Arc::clone(&self.answer),
                seen: Arc::clone(&self.seen),
            };
            Box::pin(async move { Ok(Box::new(model) as Box<dyn Loaded>) })
        }
    }

    impl Loaded for Scripted {
        fn transcribe<'a>(
            &'a mut self,
            samples: &'a [f32],
            _: &'a [String],
            language: Option<&'a str>,
        ) -> BoxFuture<'a, anyhow::Result<Heard>> {
            self.seen
                .lock()
                .unwrap()
                .push((samples.len(), language.map(str::to_string)));
            let heard = (self.answer)(samples.len());
            Box::pin(async move { heard })
        }
    }

    fn scripted(
        answer: impl Fn(usize) -> anyhow::Result<Heard> + Send + Sync + 'static,
    ) -> (LocalEngine, Seen<(usize, Option<String>)>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runtime = Scripted {
            answer: Arc::new(answer),
            seen: Arc::clone(&seen),
        };
        let engine = LocalEngine::new(
            Box::new(runtime),
            10 * SAMPLES_PER_SECOND,
            Box::new(|| false),
        );
        (engine, seen)
    }

    fn heard(text: &str, words: &[std::ops::Range<f64>]) -> anyhow::Result<Heard> {
        Ok(Heard {
            text: text.into(),
            words: words.to_vec(),
        })
    }

    /// Of 8 s of speech, the model hears the first and the last half second
    /// and nothing between them: the 7 s in the middle come back alone.
    fn skips_the_middle(len: usize) -> anyhow::Result<Heard> {
        if len == 8 * SAMPLES_PER_SECOND {
            heard("début fin", &[0.0..0.5, 7.5..8.0])
        } else if len == 7 * SAMPLES_PER_SECOND {
            heard("milieu", &[0.0..1.0])
        } else {
            anyhow::bail!("unexpected request of {len} samples")
        }
    }

    #[tokio::test]
    async fn the_dictation_language_reaches_every_piece_or_none_is_named() {
        let (engine, seen) = scripted(|_| heard("texte", &[]));
        let audio = speech(16.0, 1.0);
        engine
            .transcribe(&audio, &[], Some("fr"), &operation())
            .await
            .unwrap();
        engine
            .transcribe(&audio, &[], None, &operation())
            .await
            .unwrap();
        let languages: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        let french = Some("fr".to_string());
        assert_eq!(languages, [french.clone(), french, None, None]);
    }

    #[tokio::test]
    async fn a_stretch_the_model_skipped_is_heard_again_and_put_back() {
        let (engine, seen) = scripted(skips_the_middle);
        let text = engine
            .transcribe(&speech(8.0, 0.0), &[], Some("fr"), &operation())
            .await
            .unwrap();
        assert_eq!(text, "début milieu fin");
        // The stretch is asked in the same language, and on its own.
        let french = Some("fr".to_string());
        assert_eq!(
            *seen.lock().unwrap(),
            [
                (8 * SAMPLES_PER_SECOND, french.clone()),
                (7 * SAMPLES_PER_SECOND, french)
            ]
        );
    }

    #[tokio::test]
    async fn a_piece_the_model_heard_to_its_end_is_asked_once() {
        let (engine, seen) = scripted(|_| heard("tout entendu", &[0.0..4.0, 4.0..7.9]));
        let text = engine
            .transcribe(&speech(8.0, 0.0), &[], None, &operation())
            .await
            .unwrap();
        assert_eq!(text, "tout entendu");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_stretch_that_fails_the_second_time_keeps_the_first_text() {
        let (engine, seen) = scripted(|len| {
            if len == 7 * SAMPLES_PER_SECOND {
                anyhow::bail!("the model failed");
            }
            skips_the_middle(len)
        });
        let text = engine
            .transcribe(&speech(8.0, 0.0), &[], None, &operation())
            .await
            .unwrap();
        assert_eq!(text, "début fin");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_dictation_cancelled_during_the_second_try_stops() {
        let operation = operation();
        let cancelling = operation.clone();
        let (engine, _) = scripted(move |len| {
            if len == 7 * SAMPLES_PER_SECOND {
                cancelling.cancel();
                anyhow::bail!("the model stopped");
            }
            skips_the_middle(len)
        });
        let result = engine
            .transcribe(&speech(8.0, 0.0), &[], None, &operation)
            .await;
        assert!(result.is_err());
    }

    /// Holds the first piece until `go`, once it told `started`, and counts
    /// the models loaded and dropped.
    #[derive(Default)]
    struct Held {
        started: tokio::sync::Notify,
        go: tokio::sync::Notify,
        pieces: AtomicUsize,
        loads: AtomicUsize,
        dropped: AtomicUsize,
    }

    struct HeldRuntime(Arc<Held>);

    struct HeldModel(Arc<Held>);

    impl Runtime for HeldRuntime {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            self.0.loads.fetch_add(1, SeqCst);
            let model = HeldModel(Arc::clone(&self.0));
            Box::pin(async move { Ok(Box::new(model) as Box<dyn Loaded>) })
        }
    }

    impl Loaded for HeldModel {
        fn transcribe<'a>(
            &'a mut self,
            _: &'a [f32],
            _: &'a [String],
            _: Option<&'a str>,
        ) -> BoxFuture<'a, anyhow::Result<Heard>> {
            Box::pin(async move {
                let piece = self.0.pieces.fetch_add(1, SeqCst) + 1;
                if piece == 1 {
                    self.0.started.notify_one();
                    self.0.go.notified().await;
                }
                Ok(Heard {
                    text: format!("piece {piece}"),
                    words: Vec::new(),
                })
            })
        }
    }

    impl Drop for HeldModel {
        fn drop(&mut self) {
            self.0.dropped.fetch_add(1, SeqCst);
        }
    }

    fn held_engine() -> (LocalEngine, Arc<Held>) {
        let held = Arc::new(Held::default());
        let engine = LocalEngine::new(
            Box::new(HeldRuntime(Arc::clone(&held))),
            10 * SAMPLES_PER_SECOND,
            Box::new(|| false),
        );
        (engine, held)
    }

    #[tokio::test]
    async fn a_release_during_a_dictation_waits_for_its_last_piece() {
        let (engine, held) = held_engine();
        let operation = operation();
        let audio = speech(16.0, 1.0);
        let (text, ()) = tokio::join!(engine.transcribe(&audio, &[], None, &operation), async {
            held.started.notified().await;
            // The models were deleted while the first piece runs.
            let release = engine.release();
            held.go.notify_one();
            release.await;
        });
        assert_eq!(text.unwrap(), "piece 1 piece 2");
        assert_eq!(held.loads.load(SeqCst), 1, "the second piece loaded again");
        assert_eq!(held.dropped.load(SeqCst), 1, "kept after the dictation");
    }

    #[tokio::test]
    async fn a_dictation_cancelled_after_a_release_still_releases() {
        let (engine, held) = held_engine();
        let operation = operation();
        let audio = speech(16.0, 1.0);
        let (text, ()) = tokio::join!(engine.transcribe(&audio, &[], None, &operation), async {
            held.started.notified().await;
            let release = engine.release();
            operation.cancel();
            release.await;
        });
        assert!(text.is_err());
        assert_eq!(held.loads.load(SeqCst), 1);
        assert_eq!(held.dropped.load(SeqCst), 1, "kept after the cancel");
    }

    #[tokio::test]
    async fn a_release_between_dictations_is_at_once() {
        let (engine, held) = held_engine();
        held.go.notify_one();
        engine
            .transcribe(&speech(4.0, 0.0), &[], None, &operation())
            .await
            .unwrap();
        engine.release().await;
        assert_eq!(held.dropped.load(SeqCst), 1);
    }

    /// Maps the model file read-only, as transcribe.cpp 0.4.1 does
    /// (`CreateFileMapping` and `MapViewOfFile` on Windows), keeping it open
    /// without sharing its deletion, so no delete gets past the loaded model.
    #[cfg(target_os = "windows")]
    struct Mapping {
        model: std::path::PathBuf,
        loads: Arc<AtomicUsize>,
    }

    #[cfg(target_os = "windows")]
    struct Mapped {
        map: memmap2::Mmap,
        _file: std::fs::File,
    }

    #[cfg(target_os = "windows")]
    impl Runtime for Mapping {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            let (model, loads) = (self.model.clone(), Arc::clone(&self.loads));
            Box::pin(async move {
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 1);
                let file = options.open(model)?;
                // SAFETY: the tests never write the file while it is mapped.
                let mapped = unsafe { memmap2::Mmap::map(&file)? };
                loads.fetch_add(1, SeqCst);
                Ok(Box::new(Mapped {
                    map: mapped,
                    _file: file,
                }) as Box<dyn Loaded>)
            })
        }
    }

    #[cfg(target_os = "windows")]
    impl Loaded for Mapped {
        fn transcribe<'a>(
            &'a mut self,
            _: &'a [f32],
            _: &'a [String],
            _: Option<&'a str>,
        ) -> BoxFuture<'a, anyhow::Result<Heard>> {
            let text = format!("{} bytes", self.map.len());
            Box::pin(async move {
                Ok(Heard {
                    text,
                    words: Vec::new(),
                })
            })
        }
    }

    /// Windows may refuse to delete the file of a loaded model: deleting the
    /// models releases it first, and the next dictation loads it again once
    /// the models are back.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn deleting_the_models_releases_the_mapped_model() {
        let dir = tempfile::tempdir().unwrap();
        let (models, model) = crate::local_models::tests::ready_models(&dir.path().join("models"));
        let loads = Arc::new(AtomicUsize::new(0));
        let engine = LocalEngine::new(
            Box::new(Mapping {
                model: model.clone(),
                loads: Arc::clone(&loads),
            }),
            MAX_PIECE,
            Box::new(|| false),
        );
        engine.session.warm().await.unwrap();
        assert!(
            std::fs::remove_file(&model).is_err(),
            "Windows deleted the file of a loaded model"
        );

        crate::commands::local_models::delete(Arc::clone(&models), Some(&engine))
            .await
            .unwrap();
        assert!(!model.exists());
        assert_eq!(
            models.status().state,
            crate::local_models::LocalModelState::Missing
        );
        assert!(!engine.session.is_loaded().await);

        // No model: the dictation fails, and the engine stays usable.
        assert!(engine.session.warm().await.is_err());
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(&model, b"the model, downloaded again").unwrap();
        engine.session.warm().await.unwrap();
        assert_eq!(loads.load(SeqCst), 2);
    }
}
