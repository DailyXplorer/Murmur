use crate::actions::process_transcription_output_in;
use crate::cleanup::ProcessedText;
use crate::engine::{parakeet_languages, Engine, EngineUsed};
use crate::managers::{
    history::{
        HistoryEntry, HistoryHit, HistoryManager, HistoryVersion, PaginatedHistory,
        RECORDING_UNAVAILABLE_ERROR,
    },
    transcription::TranscriptionManager,
};
use crate::window_names::WindowContext;
use crate::{OperationId, ProcessingOperation};
use std::future::Future;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Up to 50 dictations matching `query`, best first.
#[tauri::command]
#[specta::specta]
pub async fn search_history(
    history_manager: State<'_, Arc<HistoryManager>>,
    query: String,
) -> Result<Vec<HistoryHit>, String> {
    history_manager
        .search(&query, crate::search::SEARCH_LIMIT)
        .map_err(|error| crate::search::search_failed("History", &error))
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<String, String> {
    let path = history_manager
        .get_audio_file_path(id)
        .await
        .map_err(|_| RECORDING_UNAVAILABLE_ERROR.to_string())?;
    path.to_str()
        .ok_or_else(|| RECORDING_UNAVAILABLE_ERROR.to_string())
        .map(|s| s.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

/// Deletes every dictation in the history that is not starred, and their
/// recordings. Starred dictations and meetings are kept.
#[tauri::command]
#[specta::specta]
pub async fn clear_history(history_manager: State<'_, Arc<HistoryManager>>) -> Result<(), String> {
    history_manager
        .clear_history()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Returned when the provider heard nothing in a saved recording.
pub const NO_SPEECH_ERROR: &str = "Recording contains no speech";

/// Sends a saved recording through the current provider again and stores the
/// result, with the engine that produced it, in its history entry. Any
/// failure leaves the entry as it was.
async fn retranscribe<Transcribe, Fut>(
    samples: Result<Vec<f32>, String>,
    transcribe: Transcribe,
    save: impl FnOnce(ProcessedText, EngineUsed) -> Result<HistoryEntry, String>,
) -> Result<HistoryEntry, String>
where
    Transcribe: FnOnce(Vec<f32>) -> Fut,
    Fut: Future<Output = Result<(ProcessedText, EngineUsed), String>>,
{
    let samples = samples
        .ok()
        .filter(|samples| !samples.is_empty())
        .ok_or_else(|| RECORDING_UNAVAILABLE_ERROR.to_string())?;
    let (text, source) = transcribe(samples).await?;
    if text.text.trim().is_empty() {
        return Err(NO_SPEECH_ERROR.to_string());
    }
    save(text, source)
}

/// The language of a one-off re-transcription as the engine takes it: "auto"
/// or a code such as `fr`, `yue` or `zh-Hans`. Anything else is refused before
/// any audio is read, so a bad value never reaches an engine.
fn checked_language(language: Option<String>) -> Result<Option<String>, String> {
    let Some(language) = language else {
        return Ok(None);
    };
    let well_formed = language == "auto"
        || (language.len() <= 16
            && language.split('-').enumerate().all(|(index, part)| {
                let letters = part.chars().all(|c| c.is_ascii_alphabetic());
                if index == 0 {
                    letters && (2..=3).contains(&part.len())
                } else {
                    (1..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric())
                }
            }));
    if well_formed {
        Ok(Some(language))
    } else {
        Err(format!("Invalid language: {language}"))
    }
}

/// The languages the "Transcribe again in…" menu offers for the engine the
/// user chose, or `None` when every language Murmur knows can be asked for.
/// "auto" is not listed; the menu always offers it. When the chosen engine is
/// on this computer, only what its model transcribes is offered.
fn languages_for(engine: Engine) -> Option<Vec<String>> {
    match engine {
        Engine::Local => Some(
            parakeet_languages::all()
                .iter()
                .map(|code| code.to_string())
                .collect(),
        ),
        Engine::Cloud => None,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_retranscribe_languages(app: AppHandle) -> Option<Vec<String>> {
    languages_for(crate::settings::get_settings(&app).transcription_engine)
}

/// The earlier transcripts of a dictation, newest first.
#[tauri::command]
#[specta::specta]
pub async fn get_history_versions(
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<Vec<HistoryVersion>, String> {
    history_manager.versions(id).map_err(|e| e.to_string())
}

/// Puts an earlier transcript back; the one it replaces takes its place in
/// the list of versions.
#[tauri::command]
#[specta::specta]
pub async fn restore_history_version(
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
    version_id: i64,
) -> Result<HistoryEntry, String> {
    history_manager
        .restore_version(id, version_id)
        .map_err(|e| e.to_string())
}

/// `language` is for this run only ("auto" or a code); without it the
/// language setting applies. The setting itself is never changed.
#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
    language: Option<String>,
) -> Result<HistoryEntry, String> {
    let language = checked_language(language)?;
    let samples = match history_manager.get_audio_file_path(id).await {
        Ok(path) => crate::audio_toolkit::read_wav_samples(&path).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    retranscribe(
        samples,
        move |samples| async move {
            // History retry is deliberately independent of the foreground
            // operation: cancelling a new dictation must never cancel a
            // user-requested retry.
            let operation = ProcessingOperation::new(OperationId(0));
            let transcript = transcription_manager
                .transcribe_samples_in(Arc::new(samples), operation.clone(), language.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            let (transcription, source) = (transcript.text, transcript.source);
            let pass_language = transcript.pass_language;
            // A retry is not delivered by paste, so it stays out of Insights,
            // which only counts delivered dictations.
            if transcription.is_empty() {
                return Ok((ProcessedText::unprocessed(transcription), source));
            }
            // A retry gets the same cleanup as the dictation it replaces; the
            // raw transcript stays in the entry. The window the user dictated
            // into is gone, so there is no window context to pass.
            let text = process_transcription_output_in(
                &app,
                &transcription,
                pass_language,
                WindowContext::default(),
                &operation,
                language.as_deref(),
            )
            .await;
            Ok((text, source))
        },
        |text, source| {
            history_manager
                .update_transcription(id, text, source)
                .map_err(|e| e.to_string())
        },
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    if !(1..=crate::settings::MAX_HISTORY_LIMIT).contains(&limit) {
        return Err(format!(
            "History limit must be between 1 and {} entries",
            crate::settings::MAX_HISTORY_LIMIT
        ));
    }
    crate::settings::update_settings(&app, |settings| {
        settings.history_limit = limit;
    });

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    crate::settings::update_settings(&app, |settings| {
        settings.recording_retention_period = retention_period;
    });

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn entry(text: &str) -> HistoryEntry {
        HistoryEntry {
            id: 3,
            file_name: "murmur-3.wav".to_string(),
            timestamp: 0,
            saved: true,
            title: "Recording".to_string(),
            transcription_text: text.to_string(),
            raw_text: None,
            cleanup: None,
            engine: crate::engine::Engine::Cloud,
            fallback_reason: None,
            version_count: 0,
        }
    }

    struct Calls {
        transcribed: Cell<u32>,
        saved: RefCell<Option<String>>,
    }

    impl Calls {
        fn new() -> Self {
            Self {
                transcribed: Cell::new(0),
                saved: RefCell::new(None),
            }
        }
    }

    fn retry(
        calls: &Calls,
        samples: Result<Vec<f32>, String>,
        provider: Result<&str, &str>,
    ) -> Result<HistoryEntry, String> {
        tauri::async_runtime::block_on(retranscribe(
            samples,
            |samples| async move {
                assert_eq!(samples, [0.1, -0.2]);
                calls.transcribed.set(calls.transcribed.get() + 1);
                provider
                    .map(|text| {
                        (
                            ProcessedText::unprocessed(text.to_string()),
                            EngineUsed::default(),
                        )
                    })
                    .map_err(str::to_string)
            },
            |text, _source| {
                *calls.saved.borrow_mut() = Some(text.text.clone());
                Ok(entry(&text.text))
            },
        ))
    }

    #[test]
    fn success_updates_the_entry_in_place_and_returns_the_new_text() {
        let calls = Calls::new();
        let updated = retry(&calls, Ok(vec![0.1, -0.2]), Ok("Hello again.")).unwrap();
        assert_eq!(updated.transcription_text, "Hello again.");
        assert_eq!(updated.id, 3);
        assert_eq!(calls.saved.borrow().as_deref(), Some("Hello again."));
    }

    #[test]
    fn provider_failure_leaves_the_entry_untouched() {
        let calls = Calls::new();
        let error = retry(&calls, Ok(vec![0.1, -0.2]), Err("Network error")).unwrap_err();
        assert_eq!(error, "Network error");
        assert_eq!(calls.transcribed.get(), 1);
        assert_eq!(*calls.saved.borrow(), None);
    }

    #[test]
    fn missing_or_empty_audio_is_reported_without_calling_the_provider() {
        for samples in [Err("No such file".to_string()), Ok(Vec::new())] {
            let calls = Calls::new();
            let error = retry(&calls, samples, Ok("unused")).unwrap_err();
            assert_eq!(error, RECORDING_UNAVAILABLE_ERROR);
            assert_eq!(calls.transcribed.get(), 0);
            assert_eq!(*calls.saved.borrow(), None);
        }
    }

    #[test]
    fn a_retry_saves_the_engine_that_transcribed_it() {
        use crate::engine::{Engine, FallbackReason};
        let used = EngineUsed {
            engine: Engine::Cloud,
            fallback: Some(FallbackReason::ModelNotReady),
        };
        let saved = std::cell::Cell::new(None);
        tauri::async_runtime::block_on(retranscribe(
            Ok(vec![0.1]),
            |_| async move { Ok((ProcessedText::unprocessed("Hi.".to_string()), used)) },
            |text, source| {
                saved.set(Some(source));
                Ok(entry(&text.text))
            },
        ))
        .unwrap();
        assert_eq!(saved.get(), Some(used));
    }

    #[test]
    fn a_one_off_language_is_auto_or_a_language_code() {
        assert_eq!(checked_language(None), Ok(None));
        for good in ["auto", "fr", "yue", "haw", "zh-Hans", "zh-Hant", "pt-BR"] {
            assert_eq!(
                checked_language(Some(good.to_string())),
                Ok(Some(good.to_string())),
                "{good}"
            );
        }
        for bad in [
            "",
            "f",
            "french",
            "fr_FR",
            "fr-",
            "fr--x",
            "f1",
            "../x",
            "zh-Hans-extra-long-tag",
        ] {
            assert!(checked_language(Some(bad.to_string())).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_menu_offers_the_local_models_languages_or_all_of_them() {
        let local = languages_for(Engine::Local).unwrap();
        assert_eq!(local.len(), 25);
        assert!(local.contains(&"fr".to_string()));
        assert!(!local.contains(&"ja".to_string()));
        assert!(!local.contains(&"auto".to_string()));
        assert_eq!(languages_for(Engine::Cloud), None);
    }

    #[test]
    fn silence_does_not_overwrite_the_entry() {
        let calls = Calls::new();
        let error = retry(&calls, Ok(vec![0.1, -0.2]), Ok("  ")).unwrap_err();
        assert_eq!(error, NO_SPEECH_ERROR);
        assert_eq!(*calls.saved.borrow(), None);
    }
}
