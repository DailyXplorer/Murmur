use crate::audio_toolkit::{
    apply_custom_words, detect_output_language, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence,
};
use crate::codex_transcribe;
use crate::settings::get_settings;
use anyhow::Result;
use log::{debug, error, info};
use tauri::AppHandle;

const SUPPORTED_LANGUAGES: &[&str] = &[
    "en", "fr", "de", "es", "it", "pt", "nl", "pl", "ru", "ja", "ko", "zh", "ar", "hi", "tr", "sv",
    "da", "fi", "no", "cs", "ro", "hu", "el", "uk", "vi", "th", "id", "ms", "he", "ca",
];

#[derive(Clone)]
/// Runs Murmur's single ChatGPT-session transcription pipeline.
pub struct TranscriptionManager {
    app_handle: AppHandle,
}

impl TranscriptionManager {
    /// Creates a transcription manager bound to the current Tauri app.
    pub fn new(app_handle: &AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
        }
    }

    /// Transcribes mono PCM samples and applies the configured local text
    /// normalization. An empty input produces an empty transcript.
    pub fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        #[cfg(debug_assertions)]
        if std::env::var("MURMUR_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (MURMUR_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        if audio.is_empty() {
            return Ok(String::new());
        }

        let settings = get_settings(&self.app_handle);
        let language =
            (settings.selected_language != "auto").then(|| settings.selected_language.clone());

        debug!(
            "Sending {} samples to Codex transcription (language={:?})",
            audio.len(),
            language
        );

        let text = codex_transcribe::transcribe(&audio, language.as_deref()).map_err(|err| {
            error!("Codex transcription failed: {err}");
            err
        })?;

        let mut processed = normalize_transcription_output(&text);
        processed = apply_custom_words(
            &processed,
            &settings.custom_words,
            settings.word_correction_threshold,
        );

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

        info!(
            "Codex transcription produced {} characters",
            processed.len()
        );
        Ok(processed)
    }
}
