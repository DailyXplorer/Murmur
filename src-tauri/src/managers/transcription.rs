use crate::audio_toolkit::gain::{normalize_quiet_speech, GainProfile};
use crate::audio_toolkit::normalize_transcription_output;
use crate::codex_transcribe;
use crate::settings::get_settings;
use crate::{OperationId, ProcessingOperation};
use anyhow::Result;
use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Final dictation text plus how many dictionary replacements produced it.
#[derive(Debug, Default)]
pub struct Transcript {
    pub text: String,
    pub dictionary_fixes: u32,
}

/// Sends audio to Codex cloud transcription.
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
        let language =
            (settings.selected_language != "auto").then(|| settings.selected_language.clone());
        // The saved WAV keeps the original level; only the upload is raised.
        let audio = match normalize_quiet_speech(&audio, GainProfile::new(settings.whisper_mode)) {
            Some(louder) => {
                debug!("Raised the level of quiet speech before upload");
                Arc::new(louder)
            }
            None => audio,
        };

        debug!(
            "Sending {} samples to Codex transcription (language={:?})",
            audio.len(),
            language
        );

        let text = codex_transcribe::transcribe(audio.as_slice(), language.as_deref(), &operation)
            .await
            .map_err(|err| {
                error!("Codex transcription failed: {err}");
                err
            })?;

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
            "Codex transcription produced {} characters",
            processed.len()
        );
        Ok(Transcript {
            text: processed,
            dictionary_fixes,
        })
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
