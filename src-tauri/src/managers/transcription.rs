use crate::audio_toolkit::{
    apply_custom_words, detect_output_language, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence,
};
use crate::codex_transcribe;
#[cfg(target_os = "macos")]
use crate::gemini_transcribe::GeminiTranscriber;
use crate::settings::{get_settings, TranscriptionProvider};
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
    #[cfg(target_os = "macos")]
    gemini: GeminiTranscriber,
}

impl TranscriptionManager {
    /// Creates a transcription manager bound to the current Tauri app.
    pub fn new(app_handle: &AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
            #[cfg(target_os = "macos")]
            gemini: GeminiTranscriber::new(),
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

        let provider = settings.transcription_provider;
        debug!(
            "Sending {} samples to {:?} transcription (language={:?})",
            audio.len(),
            provider,
            language
        );

        let text = match provider {
            TranscriptionProvider::Codex => {
                codex_transcribe::transcribe(&audio, language.as_deref())
            }
            TranscriptionProvider::Gemini => {
                #[cfg(target_os = "macos")]
                {
                    self.gemini.transcribe(&audio)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err(anyhow::anyhow!(
                        "Gemini transcription is currently available on macOS only."
                    ))
                }
            }
        }
        .map_err(|err| {
            error!("{provider:?} transcription failed: {err}");
            err
        })?;

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

    /// Releases provider resources before the application exits.
    pub fn shutdown(&self) {
        #[cfg(target_os = "macos")]
        self.gemini.shutdown();
    }
}
