use crate::actions::process_transcription_output;
use crate::cleanup::ProcessedText;
use crate::managers::{
    history::{HistoryEntry, HistoryManager, PaginatedHistory, RECORDING_UNAVAILABLE_ERROR},
    transcription::TranscriptionManager,
};
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

/// Returned when the provider heard nothing in a saved recording.
pub const NO_SPEECH_ERROR: &str = "Recording contains no speech";

/// Sends a saved recording through the current provider again and stores the
/// result in its history entry. Any failure leaves the entry as it was.
async fn retranscribe<Transcribe, Fut>(
    samples: Result<Vec<f32>, String>,
    transcribe: Transcribe,
    save: impl FnOnce(ProcessedText) -> Result<HistoryEntry, String>,
) -> Result<HistoryEntry, String>
where
    Transcribe: FnOnce(Vec<f32>) -> Fut,
    Fut: Future<Output = Result<ProcessedText, String>>,
{
    let samples = samples
        .ok()
        .filter(|samples| !samples.is_empty())
        .ok_or_else(|| RECORDING_UNAVAILABLE_ERROR.to_string())?;
    let text = transcribe(samples).await?;
    if text.text.trim().is_empty() {
        return Err(NO_SPEECH_ERROR.to_string());
    }
    save(text)
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<HistoryEntry, String> {
    let samples = match history_manager.get_audio_file_path(id).await {
        Ok(path) => crate::audio_toolkit::read_wav_samples(&path).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let provider = crate::settings::get_settings(&app).transcription_provider;
    retranscribe(
        samples,
        move |samples| async move {
            // History retry is deliberately independent of the foreground
            // operation: cancelling a new dictation must never cancel a
            // user-requested retry.
            let operation = ProcessingOperation::new(OperationId(0));
            let transcription = transcription_manager
                .transcribe_with_provider(Arc::new(samples), operation.clone(), provider)
                .await
                .map_err(|e| e.to_string())?
                .text;
            // A retry is not delivered by paste, so it stays out of Insights,
            // which only counts delivered dictations.
            if transcription.is_empty() {
                return Ok(ProcessedText::unprocessed(transcription));
            }
            // A retry gets the same cleanup as the dictation it replaces; the
            // raw transcript stays in the entry. The window the user dictated
            // into is gone, so there are no window names to pass.
            Ok(
                process_transcription_output(
                    &app,
                    &transcription,
                    provider,
                    Vec::new(),
                    &operation,
                )
                .await,
            )
        },
        |text| {
            history_manager
                .update_transcription(id, text)
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
    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings);

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

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(&app, settings);

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
                    .map(|text| ProcessedText::unprocessed(text.to_string()))
                    .map_err(str::to_string)
            },
            |text| {
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
    fn silence_does_not_overwrite_the_entry() {
        let calls = Calls::new();
        let error = retry(&calls, Ok(vec![0.1, -0.2]), Ok("  ")).unwrap_err();
        assert_eq!(error, NO_SPEECH_ERROR);
        assert_eq!(*calls.saved.borrow(), None);
    }
}
