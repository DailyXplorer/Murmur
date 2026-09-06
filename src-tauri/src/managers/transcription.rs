use crate::audio_toolkit::{
    apply_custom_words, detect_output_language, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence,
};
use crate::codex_transcribe;
use crate::gemini_transcribe::{GeminiTranscriber, ShutdownReceipt};
use crate::settings::{get_settings, TranscriptionProvider};
use crate::{OperationId, ProcessingOperation};
use anyhow::Result;
use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

const SUPPORTED_LANGUAGES: &[&str] = &[
    "en", "fr", "de", "es", "it", "pt", "nl", "pl", "ru", "ja", "ko", "zh", "ar", "hi", "tr", "sv",
    "da", "fi", "no", "cs", "ro", "hu", "el", "uk", "vi", "th", "id", "ms", "he", "ca",
];

/// Routes audio through the selected cloud transcription provider.
pub struct TranscriptionManager {
    app_handle: AppHandle,
    gemini: GeminiTranscriber,
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
            gemini: GeminiTranscriber::new(),
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
        let provider = get_settings(&self.app_handle).transcription_provider;
        self.transcribe_with_provider(Arc::new(audio), operation, provider)
            .await
    }

    /// Runs a request against the provider chosen when that request starts.
    /// UI changes while the network call is pending must not make downstream
    /// output processing describe a different provider than the one used.
    pub(crate) async fn transcribe_with_provider(
        &self,
        audio: Arc<Vec<f32>>,
        operation: ProcessingOperation,
        provider: TranscriptionProvider,
    ) -> Result<String> {
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
            return Ok(String::new());
        }
        if operation.is_cancelled() {
            return Err(anyhow::anyhow!("Transcription cancelled"));
        }

        let settings = get_settings(&self.app_handle);
        let language =
            (settings.selected_language != "auto").then(|| settings.selected_language.clone());

        debug!(
            "Sending {} samples to {:?} transcription (language={:?})",
            audio.len(),
            provider,
            language
        );

        let text = match provider {
            TranscriptionProvider::Codex => {
                codex_transcribe::transcribe(audio.as_slice(), language.as_deref(), &operation)
                    .await
            }
            TranscriptionProvider::Gemini => {
                let gemini = self.gemini.clone();
                let operation = operation.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    gemini.transcribe(audio.as_slice(), &operation)
                })
                .await
                .map_err(|error| anyhow::anyhow!("Gemini transcription worker panicked: {error}"))?
            }
        }
        .map_err(|err| {
            error!("{provider:?} transcription failed: {err}");
            err
        })?;

        if operation.is_cancelled() {
            return Err(anyhow::anyhow!("Transcription cancelled"));
        }

        let mut processed = normalize_transcription_output(&text);
        processed = apply_custom_words(
            &processed,
            &settings.custom_words,
            settings.word_correction_threshold,
        );

        if provider == TranscriptionProvider::Codex {
            let language_evidence = if settings.selected_language == "auto" {
                let supported_languages = SUPPORTED_LANGUAGES
                    .iter()
                    .map(|language| language.to_string())
                    .collect::<Vec<_>>();
                detect_output_language(&processed, &supported_languages)
                    .map(OutputLanguageEvidence::TextDetected)
                    .unwrap_or(OutputLanguageEvidence::Unknown)
            } else {
                OutputLanguageEvidence::UserSelected(settings.selected_language.clone())
            };
            processed = remove_filler_words(
                &processed,
                &language_evidence,
                &settings.custom_filler_words,
                settings.filler_word_removal_enabled,
            );
        }

        info!(
            "{provider:?} transcription produced {} characters",
            processed.len()
        );
        Ok(processed)
    }

    /// Keeps CLI calls synchronous and independent of desktop cancellation.
    pub fn transcribe_sync(&self, audio: Vec<f32>) -> Result<String> {
        let operation = ProcessingOperation::new(OperationId(0));
        tauri::async_runtime::block_on(self.transcribe(audio, operation))
    }

    /// Signals all active transports and starts off-main Gemini cleanup.
    /// The returned receipt is required by callers that are about to end the
    /// process, so a Murmur-owned local server is not orphaned on exit.
    pub fn begin_shutdown(&self) -> ShutdownReceipt {
        // This registry is independent of Gemini's runtime-state mutex.
        // Exit can therefore interrupt Codex HTTP and a Gemini gRPC request
        // before any provider cleanup tries to acquire that mutex.
        self.shutdown_requested.store(true, Ordering::Release);
        let active_operations = self
            .active_operations
            .lock()
            .map(|operations| operations.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for operation in active_operations {
            operation.cancel();
        }
        self.gemini.begin_shutdown()
    }

    /// Releases provider resources without waiting. Tauri's terminal Exit
    /// event uses this as a fallback; normal exit waits through the foreground
    /// coordinator drain before reaching that event.
    pub fn shutdown(&self) {
        let _ = self.begin_shutdown();
    }

    /// Bounded cleanup for non-AppKit callers such as the headless CLI worker.
    pub fn shutdown_and_wait(&self, timeout: std::time::Duration) -> bool {
        self.begin_shutdown().wait_bounded(timeout)
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
