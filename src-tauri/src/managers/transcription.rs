use crate::audio_toolkit::gain::{normalize_quiet_speech, GainProfile};
use crate::audio_toolkit::normalize_transcription_output;
use crate::codex_transcribe;
use crate::engine::local::LocalEngine;
use crate::engine::{self, Conditions, Engine, EngineUsed, Route};
use crate::local_models::LocalModels;
use crate::settings::{get_settings, AppSettings};
use crate::{OperationId, ProcessingOperation};
use anyhow::Result;
use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

/// Final dictation text plus how many dictionary replacements produced it.
#[derive(Debug, Default)]
pub struct Transcript {
    pub text: String,
    pub dictionary_fixes: u32,
    pub source: EngineUsed,
}

/// Transcribes `audio` with `engine`. `language` is the dictation setting.
async fn run(
    engine: Engine,
    local: Option<&LocalEngine>,
    audio: &[f32],
    language: &str,
    operation: &ProcessingOperation,
) -> Result<String> {
    match engine {
        Engine::Cloud => {
            let language = (language != "auto").then_some(language);
            debug!(
                "Sending {} samples to Codex transcription (language={:?})",
                audio.len(),
                language
            );
            codex_transcribe::transcribe(audio, language, operation)
                .await
                .inspect_err(|err| error!("Codex transcription failed: {err}"))
        }
        Engine::Local => {
            let local = local.ok_or_else(|| anyhow::anyhow!("No local engine in this build"))?;
            debug!("Transcribing {} samples on this Mac", audio.len());
            local.transcribe(audio, operation).await
        }
    }
}

/// Sends audio to the local engine or Codex.
pub struct TranscriptionManager {
    app_handle: AppHandle,
    active_operations: Arc<Mutex<HashMap<u64, ProcessingOperation>>>,
    next_active_operation: AtomicU64,
    shutdown_requested: AtomicBool,
}

struct ActiveOperationGuard {
    id: u64,
    active_operations: Arc<Mutex<HashMap<u64, ProcessingOperation>>>,
}

impl Drop for ActiveOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.active_operations.lock() {
            operations.remove(&self.id);
        }
    }
}

impl TranscriptionManager {
    /// Creates a transcription manager bound to the current Tauri app.
    pub fn new(app_handle: &AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
            active_operations: Arc::new(Mutex::new(HashMap::new())),
            next_active_operation: AtomicU64::new(1),
            shutdown_requested: AtomicBool::new(false),
        }
    }

    /// Transcribes mono PCM samples and applies the configured local text
    /// normalization. An empty input produces an empty transcript.
    pub async fn transcribe(
        &self,
        audio: Vec<f32>,
        operation: ProcessingOperation,
    ) -> Result<String> {
        self.transcribe_samples(Arc::new(audio), operation)
            .await
            .map(|transcript| transcript.text)
    }

    /// Transcribes shared samples and corrects them with the dictionary.
    pub(crate) async fn transcribe_samples(
        &self,
        audio: Arc<Vec<f32>>,
        operation: ProcessingOperation,
    ) -> Result<Transcript> {
        let _active_operation = self.register_active_operation(operation.clone());
        if self.shutdown_requested.load(Ordering::Acquire) {
            operation.cancel();
            return Err(anyhow::anyhow!("Transcription manager is shutting down"));
        }
        #[cfg(debug_assertions)]
        if std::env::var("MURMUR_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (MURMUR_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        if audio.is_empty() {
            return Ok(Transcript::default());
        }
        if operation.is_cancelled() {
            return Err(anyhow::anyhow!("Transcription cancelled"));
        }

        let settings = get_settings(&self.app_handle);
        let conditions = self.conditions(&settings);
        // The saved WAV keeps the original level; only the engine's input is
        // raised.
        let audio = match normalize_quiet_speech(&audio, GainProfile::new(settings.whisper_mode)) {
            Some(louder) => {
                debug!("Raised the level of quiet speech before transcription");
                Arc::new(louder)
            }
            None => audio,
        };

        let local = self.app_handle.try_state::<Arc<LocalEngine>>();
        let local = local.as_deref().map(Arc::as_ref);
        let (text, source) = engine::transcribe(
            settings.transcription_engine,
            conditions,
            || operation.is_cancelled(),
            |engine| run(engine, local, &audio, conditions.language, &operation),
        )
        .await?;

        if operation.is_cancelled() {
            return Err(anyhow::anyhow!("Transcription cancelled"));
        }

        let (processed, dictionary_fixes) = crate::dictionary::correct(
            &normalize_transcription_output(&text),
            &settings.custom_words,
            &settings.learned_words,
            settings.word_correction_threshold,
        );

        info!(
            "The {} engine produced {} characters",
            source.engine.as_str(),
            processed.len()
        );
        Ok(Transcript {
            text: processed,
            dictionary_fixes,
            source,
        })
    }

    fn conditions<'a>(&self, settings: &'a AppSettings) -> Conditions<'a> {
        let model_ready = self
            .app_handle
            .try_state::<Arc<LocalModels>>()
            .is_some_and(|models| models.is_ready())
            && self.app_handle.try_state::<Arc<LocalEngine>>().is_some();
        Conditions {
            model_ready,
            signed_in: crate::codex_auth::auth_status().signed_in,
            language: &settings.selected_language,
        }
    }

    /// At the start of a recording: loads the local model when the dictation
    /// will go to it, so the speaking time hides the load.
    pub fn warm_if_local(&self) {
        let settings = get_settings(&self.app_handle);
        let route = engine::route(settings.transcription_engine, self.conditions(&settings));
        if matches!(
            route,
            Route::Run(EngineUsed {
                engine: Engine::Local,
                ..
            })
        ) {
            if let Some(local) = self.app_handle.try_state::<Arc<LocalEngine>>() {
                local.warm();
            }
        }
    }

    /// Keeps CLI calls synchronous and independent of desktop cancellation.
    pub fn transcribe_sync(&self, audio: Vec<f32>) -> Result<String> {
        let operation = ProcessingOperation::new(OperationId(0));
        tauri::async_runtime::block_on(self.transcribe(audio, operation))
    }

    /// Cancels every active request and refuses new ones, without waiting.
    pub fn shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
        let active_operations = self
            .active_operations
            .lock()
            .map(|operations| operations.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for operation in active_operations {
            operation.cancel();
        }
    }

    fn register_active_operation(&self, operation: ProcessingOperation) -> ActiveOperationGuard {
        let id = self.next_active_operation.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut operations) = self.active_operations.lock() {
            operations.insert(id, operation);
        }
        ActiveOperationGuard {
            id,
            active_operations: Arc::clone(&self.active_operations),
        }
    }
}
