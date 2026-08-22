use crate::audio_toolkit::{
    apply_custom_words, detect_output_language, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence,
};
use crate::codex_transcribe;
use crate::managers::model::{ModelManager, CODEX_MODEL_ID};
use crate::settings::get_settings;
use anyhow::Result;
use log::{debug, error, info};
use serde::Serialize;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize)]
pub struct ModelStateEvent {
    pub event_type: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

pub struct LoadingGuard {
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
}

impl Drop for LoadingGuard {
    fn drop(&mut self) {
        let mut is_loading = match self.is_loading.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *is_loading = false;
        self.loading_condvar.notify_all();
    }
}

#[derive(Clone)]
pub struct TranscriptionManager {
    model_manager: Arc<ModelManager>,
    app_handle: AppHandle,
    current_model_id: Arc<Mutex<Option<String>>>,
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
}

impl TranscriptionManager {
    pub fn new(app_handle: &AppHandle, model_manager: Arc<ModelManager>) -> Result<Self> {
        Ok(Self {
            model_manager,
            app_handle: app_handle.clone(),
            current_model_id: Arc::new(Mutex::new(Some(CODEX_MODEL_ID.to_string()))),
            is_loading: Arc::new(Mutex::new(false)),
            loading_condvar: Arc::new(Condvar::new()),
        })
    }

    pub fn is_model_loaded(&self) -> bool {
        true
    }

    pub fn try_start_loading(&self) -> Option<LoadingGuard> {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading {
            return None;
        }
        *is_loading = true;
        Some(LoadingGuard {
            is_loading: self.is_loading.clone(),
            loading_condvar: self.loading_condvar.clone(),
        })
    }

    pub fn load_model(&self, _model_id: &str) -> Result<()> {
        self.mark_codex_ready();
        Ok(())
    }

    pub fn load_model_with_device(
        &self,
        model_id: &str,
        _device_index: Option<usize>,
    ) -> Result<()> {
        self.load_model(model_id)
    }

    pub fn initiate_model_load(&self) {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading {
            return;
        }
        *is_loading = true;
        drop(is_loading);
        let manager = self.clone();
        thread::spawn(move || {
            manager.mark_codex_ready();
            let mut is_loading = manager.is_loading.lock().unwrap();
            *is_loading = false;
            manager.loading_condvar.notify_all();
        });
    }

    pub fn get_current_model(&self) -> Option<String> {
        self.current_model_id
            .lock()
            .unwrap()
            .clone()
            .or_else(|| Some(CODEX_MODEL_ID.to_string()))
    }

    pub fn current_backend(&self) -> Option<String> {
        Some("codex-cloud".to_string())
    }

    pub fn transcribe(&self, audio: Vec<f32>) -> Result<String> {
        #[cfg(debug_assertions)]
        if std::env::var("HANDY_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (HANDY_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        if audio.is_empty() {
            return Ok(String::new());
        }

        let settings = get_settings(&self.app_handle);
        let language = if settings.selected_language == "auto" {
            None
        } else {
            Some(settings.selected_language.clone())
        };

        debug!(
            "Sending {} samples to Codex transcription (language={:?})",
            audio.len(),
            language
        );

        let text = match codex_transcribe::transcribe(&audio, language.as_deref()) {
            Ok(text) => text,
            Err(err) => {
                error!("Codex transcription failed: {err}");
                return Err(err);
            }
        };

        let mut processed = normalize_transcription_output(&text);
        processed = apply_custom_words(
            &processed,
            &settings.custom_words,
            settings.word_correction_threshold,
        );
        let language_evidence = if settings.selected_language == "auto" {
            let supported_languages = self
                .model_manager
                .get_model_info(CODEX_MODEL_ID)
                .map(|model| model.supported_languages)
                .unwrap_or_default();
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

    fn mark_codex_ready(&self) {
        *self.current_model_id.lock().unwrap() = Some(CODEX_MODEL_ID.to_string());
        let name = self
            .model_manager
            .get_model_info(CODEX_MODEL_ID)
            .map(|info| info.name)
            .unwrap_or_else(|| "Codex".to_string());
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_completed".to_string(),
                model_id: Some(CODEX_MODEL_ID.to_string()),
                model_name: Some(name),
                error: None,
            },
        );
    }
}

pub fn describe_compute_devices() -> Vec<String> {
    vec!["codex-cloud".to_string()]
}
