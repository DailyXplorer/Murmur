use crate::audio_toolkit::{
    apply_custom_words, detect_output_language, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence,
};
use crate::codex_transcribe;
use crate::gemini_transcribe::GeminiTranscriber;
use crate::settings::{get_settings, TranscriptionProvider};
use crate::{OperationId, ProcessingOperation};
use anyhow::Result;
use log::{debug, error, info};
use tauri::AppHandle;

const SUPPORTED_LANGUAGES: &[&str] = &[
    "en", "fr", "de", "es", "it", "pt", "nl", "pl", "ru", "ja", "ko", "zh", "ar", "hi", "tr", "sv",
    "da", "fi", "no", "cs", "ro", "hu", "el", "uk", "vi", "th", "id", "ms", "he", "ca",
];

/// Routes audio through the selected cloud transcription provider.
pub struct TranscriptionManager {
    app_handle: AppHandle,
    gemini: GeminiTranscriber,
}

impl TranscriptionManager {
    /// Creates a transcription manager bound to the current Tauri app.
    pub fn new(app_handle: &AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
            gemini: GeminiTranscriber::new(),
        }
    }

    /// Transcribes mono PCM samples and applies the configured local text
    /// normalization. An empty input produces an empty transcript.
    pub async fn transcribe(
        &self,
        audio: Vec<f32>,
        operation: ProcessingOperation,
    ) -> Result<String> {
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

        let provider = settings.transcription_provider;
        debug!(
            "Sending {} samples to {:?} transcription (language={:?})",
            audio.len(),
            provider,
            language
        );

        let text = match provider {
            TranscriptionProvider::Codex => {
                codex_transcribe::transcribe(&audio, language.as_deref(), &operation).await
            }
            TranscriptionProvider::Gemini => {
                let gemini = self.gemini.clone();
                let audio = audio.clone();
                let operation = operation.clone();
                tauri::async_runtime::spawn_blocking(move || gemini.transcribe(&audio, &operation))
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Gemini transcription worker panicked: {error}")
                    })?
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

    /// CLI callers intentionally run without a cancellable desktop operation.
    /// The adapter keeps the command-line contract synchronous without
    /// reintroducing a blocking network path into the desktop pipeline.
    pub fn transcribe_sync(&self, audio: Vec<f32>) -> Result<String> {
        let operation = ProcessingOperation::new(OperationId(0));
        tauri::async_runtime::block_on(self.transcribe(audio, operation))
    }

    /// Releases provider resources before the application exits.
    pub fn shutdown(&self) {
        self.gemini.shutdown();
    }
}
