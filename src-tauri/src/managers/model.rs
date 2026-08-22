use crate::settings::{get_settings, write_settings};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

pub const CODEX_MODEL_ID: &str = "codex";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum EngineType {
    Codex,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub engine_type: EngineType,
    pub is_downloaded: bool,
    pub supported_languages: Vec<String>,
    pub supports_language_selection: bool,
    pub supports_language_detection: bool,
}

pub fn effective_language(
    intent: &str,
    supported_languages: &[String],
    supports_language_detection: bool,
) -> String {
    if intent == "zh-Hans" || intent == "zh-Hant" {
        return intent.to_string();
    }
    if intent != "auto"
        && supported_languages.iter().any(|language| {
            language == intent || language.split('-').next() == intent.split('-').next()
        })
    {
        return intent.to_string();
    }
    if supports_language_detection || supported_languages.is_empty() {
        return if intent.is_empty() {
            "auto".to_string()
        } else {
            intent.to_string()
        };
    }
    supported_languages
        .first()
        .cloned()
        .unwrap_or_else(|| "auto".to_string())
}

#[derive(Clone)]
pub struct ModelManager {
    app_handle: AppHandle,
}

impl ModelManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let manager = Self {
            app_handle: app_handle.clone(),
        };
        manager.migrate_selection_to_codex();
        Ok(manager)
    }

    fn migrate_selection_to_codex(&self) {
        let mut settings = get_settings(&self.app_handle);
        if settings.selected_model == CODEX_MODEL_ID {
            return;
        }
        settings.selected_model = CODEX_MODEL_ID.to_string();
        write_settings(&self.app_handle, settings);
    }

    pub fn get_available_models(&self) -> Vec<ModelInfo> {
        vec![codex_model()]
    }

    pub fn get_model_info(&self, model_id: &str) -> Option<ModelInfo> {
        if model_id == CODEX_MODEL_ID || model_id.is_empty() {
            Some(codex_model())
        } else {
            None
        }
    }
}

fn codex_model() -> ModelInfo {
    ModelInfo {
        id: CODEX_MODEL_ID.to_string(),
        name: "Codex".to_string(),
        description: "Cloud transcription through your Codex / ChatGPT session.".to_string(),
        engine_type: EngineType::Codex,
        is_downloaded: true,
        supported_languages: CODEX_LANGUAGES
            .iter()
            .map(|code| code.to_string())
            .collect(),
        supports_language_selection: true,
        supports_language_detection: true,
    }
}

const CODEX_LANGUAGES: &[&str] = &[
    "en", "fr", "de", "es", "it", "pt", "nl", "pl", "ru", "ja", "ko", "zh", "ar", "hi", "tr", "sv",
    "da", "fi", "no", "cs", "ro", "hu", "el", "uk", "vi", "th", "id", "ms", "he", "ca",
];
