//! Parakeet TDT 0.6B v3 on this Mac. Apple silicon runs the Core ML models
//! in the meetings helper (`fluid`), other Macs the GGUF model on the CPU in
//! the app (`cpu`). Both get pieces short enough for one pass, cut at pauses.

#[cfg(target_arch = "x86_64")]
mod cpu;
#[cfg(target_arch = "aarch64")]
mod fluid;
#[cfg(test)]
mod real_engine;
pub mod session;

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::pauses::{is_silent, split_at_pauses};
use crate::local_models::LocalModels;
use crate::ProcessingOperation;
use session::{Runtime, Session};
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

    /// Transcribes 16 kHz mono audio of any length. Silence gives no text
    /// and never reaches the model, which would invent words.
    pub async fn transcribe(
        &self,
        samples: &[f32],
        operation: &ProcessingOperation,
    ) -> anyhow::Result<String> {
        #[cfg(debug_assertions)]
        if std::env::var("MURMUR_FORCE_LOCAL_FAILURE").is_ok() {
            anyhow::bail!("Simulated local transcription failure (MURMUR_FORCE_LOCAL_FAILURE)");
        }
        if is_silent(samples) {
            return Ok(String::new());
        }
        let mut texts = Vec::new();
        for range in split_at_pauses(samples, self.max_piece) {
            let piece = padded(&samples[range]);
            let limit = RUN_ALLOWANCE + seconds(piece.len());
            let text = tokio::select! {
                biased;
                () = operation.cancelled() => anyhow::bail!("Local transcription cancelled"),
                text = self.session.transcribe(&piece, limit) => text?,
            };
            texts.push(text.trim().to_string());
        }
        Ok(join(texts))
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
    let scratch = app.path().app_cache_dir().ok()?.join("local-stt");
    Some(Box::new(fluid::FluidRuntime::new(
        program,
        models.dir().to_path_buf(),
        scratch,
    )))
}

#[cfg(not(target_arch = "aarch64"))]
fn runtime(_app: &AppHandle, models: &LocalModels) -> Option<Box<dyn Runtime>> {
    Some(Box::new(cpu::CpuRuntime {
        model: models
            .dir()
            .join(crate::local_models::manifest::gguf_path()),
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

/// Registers the dictation engine, after the local models.
pub fn init(app: &AppHandle) {
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

#[cfg(test)]
mod tests {
    use super::session::{BoxFuture, Loaded};
    use super::*;
    use crate::OperationId;
    use std::sync::Mutex;

    /// Answers each piece with its length in samples, or the given text.
    struct Lengths {
        pieces: Arc<Mutex<Vec<usize>>>,
        answer: Option<&'static str>,
    }

    impl Runtime for Lengths {
        fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
            let model = Lengths {
                pieces: Arc::clone(&self.pieces),
                answer: self.answer,
            };
            Box::pin(async move { Ok(Box::new(model) as Box<dyn Loaded>) })
        }
    }

    impl Loaded for Lengths {
        fn transcribe<'a>(
            &'a mut self,
            samples: &'a [f32],
        ) -> BoxFuture<'a, anyhow::Result<String>> {
            self.pieces.lock().unwrap().push(samples.len());
            let text = self
                .answer
                .map_or_else(|| format!(" {} ", samples.len()), str::to_string);
            Box::pin(async move { Ok(text) })
        }
    }

    fn engine(answer: Option<&'static str>) -> (LocalEngine, Arc<Mutex<Vec<usize>>>) {
        let pieces = Arc::new(Mutex::new(Vec::new()));
        let runtime = Lengths {
            pieces: Arc::clone(&pieces),
            answer,
        };
        let engine = LocalEngine::new(
            Box::new(runtime),
            10 * SAMPLES_PER_SECOND,
            Box::new(|| false),
        );
        (engine, pieces)
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
        let text = engine.transcribe(&audio, &operation()).await.unwrap();
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
            .transcribe(&speech(4.0, 0.0), &operation())
            .await
            .unwrap();
        engine
            .transcribe(&speech(0.25, 0.0), &operation())
            .await
            .unwrap();
        assert_eq!(*pieces.lock().unwrap(), [4 * SAMPLES_PER_SECOND, MIN_PIECE]);
    }

    #[tokio::test]
    async fn silence_never_reaches_the_model() {
        let (engine, pieces) = engine(Some("Thank you."));
        let text = engine
            .transcribe(&vec![0.0; 5 * SAMPLES_PER_SECOND], &operation())
            .await
            .unwrap();
        assert_eq!(text, "");
        assert!(pieces.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_dictation_stops() {
        let (engine, _) = engine(None);
        let cancelled = operation();
        cancelled.cancel();
        assert!(engine
            .transcribe(&speech(4.0, 0.0), &cancelled)
            .await
            .is_err());
    }
}
