use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, OverlayStyle, TranscriptionProvider};
use crate::shortcut;
use crate::tray::{change_tray_icon, TrayIconState};
use crate::utils::{self, show_recording_overlay, show_transcribing_overlay};
use crate::{ProcessingOperation, TranscriptionCoordinator};
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::{debug, error};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, Emitter};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

/// Drop guard that finishes the cancellation lifecycle when the transcription
/// pipeline completes, even if the task unwinds.
struct FinishGuard {
    app: AppHandle,
    operation: ProcessingOperation,
}
impl Drop for FinishGuard {
    fn drop(&mut self) {
        if let Some(c) = self.app.try_state::<TranscriptionCoordinator>() {
            c.notify_processing_finished(self.operation.id());
        }
    }
}

/// Owns a newly-created recording until a history row commits it.
///
/// A pipeline cancellation, save failure, or panic drops this value and must
/// remove the unreferenced file. `commit` transfers ownership to history.
struct PendingWav {
    path: PathBuf,
    committed: bool,
}

impl PendingWav {
    /// Reserves a new path before asynchronous WAV writing begins.
    ///
    /// Exclusive creation makes the guard the sole owner of the path. If an
    /// existing history row refers to it, reservation fails and that file is
    /// left untouched.
    fn reserve(path: PathBuf) -> std::io::Result<Self> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(&mut self) {
        self.committed = true;
    }

    /// Transfers deletion ownership to the blocking WAV writer. If the async
    /// pipeline is cancelled or dropped while that writer still runs, this
    /// guard remains alive in the worker and cannot remove the path underneath
    /// an active `WavWriter`.
    fn write(
        self,
        samples: Arc<Vec<f32>>,
    ) -> tauri::async_runtime::JoinHandle<(Self, anyhow::Result<()>)> {
        let path = self.path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let result = crate::audio_toolkit::save_wav_file(&path, samples.as_slice());
            (self, result)
        })
    }
}

impl Drop for PendingWav {
    fn drop(&mut self) {
        if self.committed {
            return;
        }

        match fs::remove_file(&self.path) {
            Ok(()) => debug!("Removed uncommitted recording: {}", self.path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => error!(
                "Failed to remove uncommitted recording {}: {}",
                self.path.display(),
                error
            ),
        }
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(
        &self,
        app: &AppHandle,
        binding_id: &str,
        shortcut_str: &str,
        operation: ProcessingOperation,
    );
}

// Transcribe Action
struct TranscribeAction;

async fn complete_unless_cancelled<F, C>(operation: F, is_cancelled: C) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}

fn operation_was_cancelled(
    operation: &ProcessingOperation,
    recording_manager: &AudioRecordingManager,
    cancel_generation: u64,
) -> bool {
    operation.is_cancelled() || recording_manager.was_cancelled_since(cancel_generation)
}

/// History follows the irreversible paste boundary, not a later cancellation
/// request. `Skipped` retains the completed text for recovery when its output
/// failed, including a clipboard-only write failure.
fn paste_outcome_commits_history(outcome: crate::clipboard::PasteOutcome) -> bool {
    outcome != crate::clipboard::PasteOutcome::Cancelled
}

async fn maybe_convert_chinese_variant(
    effective_language: &str,
    transcription: &str,
) -> Option<String> {
    // Gate conversion on the explicitly selected Chinese variant so automatic
    // detection never rewrites characters in Japanese or other CJK output.
    let is_simplified = effective_language == "zh-Hans";
    let is_traditional = effective_language == "zh-Hant";

    if !is_simplified && !is_traditional {
        debug!("effective language is not Simplified or Traditional Chinese; skipping conversion");
        return None;
    }

    debug!(
        "Starting Chinese variant conversion using OpenCC for language: {}",
        effective_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2tw
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

fn output_language_for_provider(
    provider: TranscriptionProvider,
    selected_language: &str,
) -> Option<&str> {
    (provider == TranscriptionProvider::Codex).then_some(selected_language)
}

pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    provider: TranscriptionProvider,
) -> String {
    let settings = get_settings(app);
    let mut final_text = transcription.to_string();

    // Gemini does not accept Murmur's language preference. Preserve the
    // stored selection for a later Codex dictation, but do not let an old
    // Chinese-variant preference rewrite Gemini output.
    if let Some(effective_language) =
        output_language_for_provider(provider, &settings.selected_language)
    {
        if let Some(converted_text) =
            maybe_convert_chinese_variant(effective_language, transcription).await
        {
            final_text = converted_text;
        }
    }

    final_text
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);

        let rm = app.state::<Arc<AudioRecordingManager>>();
        if rm.is_stopping() {
            // The coordinator retries this press after the prior stop releases
            // the recorder; recording UI must wait for admission.
            debug!("Deferring recording start while a prior stop is active");
            return;
        }

        let kickoff_started = Instant::now();
        let rm_clone = Arc::clone(&rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.ensure_recorder() {
                debug!("Recorder pre-load failed: {}", e);
            }
        });
        let kickoff_elapsed = kickoff_started.elapsed();

        let binding_id = binding_id.to_string();
        let tray_started = Instant::now();
        change_tray_icon(app, TrayIconState::Recording);
        let tray_elapsed = tray_started.elapsed();

        // Get the microphone mode to determine audio feedback timing
        let plan_started = Instant::now();
        let settings = get_settings(app);
        let is_always_on = settings.always_on_microphone;
        let plan_elapsed = plan_started.elapsed();

        let overlay_started = Instant::now();
        match settings.overlay_style {
            OverlayStyle::Minimal => show_recording_overlay(app),
            OverlayStyle::None => {}
        }
        // Everything above runs before capture can begin, so each span here is
        // added keypress->capture latency.
        debug!(
            "start-path pre-recording steps: recorder_kickoff={:?} tray={:?} settings+stream_plan={:?} overlay={:?}",
            kickoff_elapsed,
            tray_elapsed,
            plan_elapsed,
            overlay_started.elapsed()
        );
        debug!("Microphone mode - always_on: {}", is_always_on);

        let mut recording_error: Option<String> = None;
        let recording_start_time = Instant::now();
        match rm.try_start_recording(&binding_id) {
            Ok(readiness) => {
                debug!(
                    "Recording request accepted in {:?}; waiting for first microphone samples",
                    recording_start_time.elapsed()
                );
                let generation = readiness.generation();
                let app_clone = app.clone();
                let rm_clone = Arc::clone(&rm);
                std::thread::spawn(move || {
                    if !readiness.wait() {
                        debug!("Microphone readiness wait ended without receiving samples");
                        return;
                    }

                    // Development-only preview hook for evaluating the brief
                    // arming animation on hardware that normally starts too fast
                    // to make it visible.
                    #[cfg(debug_assertions)]
                    if let Ok(delay_ms) = std::env::var("MURMUR_DEBUG_MIC_READY_DELAY_MS")
                        .unwrap_or_default()
                        .parse::<u64>()
                    {
                        let delay_ms = delay_ms.min(10_000);
                        if delay_ms > 0 {
                            debug!("Delaying microphone-ready cue by {delay_ms}ms for UI preview");
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }

                    if !rm_clone.is_recording_readiness_current(generation) {
                        debug!("Microphone became ready for an inactive recording");
                        return;
                    }

                    debug!("Microphone is receiving samples; recording is ready");
                    utils::emit_recording_ready(&app_clone);

                    // The start chime is a readiness cue, so it must follow the
                    // first real input callback rather than Stream::play() or a
                    // fixed delay. The helper returns immediately when feedback
                    // is disabled; mute still follows the same readiness point.
                    if rm_clone.is_recording_readiness_current(generation) {
                        play_feedback_sound_blocking(&app_clone, SoundType::Start);
                    }
                    if rm_clone.is_recording_readiness_current(generation) {
                        rm_clone.apply_mute();
                    }
                });
            }
            Err(e) => {
                debug!("Failed to start recording: {}", e);
                recording_error = Some(e);
            }
        }

        if recording_error.is_none() {
            // Dynamically register the cancel shortcut in a separate task to avoid deadlock
            shortcut::register_cancel_shortcut(app);
        } else {
            // Starting failed (for example due to blocked microphone permissions).
            // Revert UI state so we don't stay stuck in the recording overlay.
            utils::hide_recording_overlay(app);
            change_tray_icon(app, TrayIconState::Idle);
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                let _ = app.emit(
                    "recording-error",
                    RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err),
                    },
                );
            }
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(
        &self,
        app: &AppHandle,
        binding_id: &str,
        _shortcut_str: &str,
        operation: ProcessingOperation,
    ) {
        // Prevent a slow microphone from emitting a ready event or start chime
        // after the user has already requested stop.
        app.state::<Arc<AudioRecordingManager>>()
            .invalidate_recording_readiness();

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

        change_tray_icon(app, TrayIconState::Transcribing);
        show_transcribing_overlay(app);

        // Unmute before playing audio feedback so the stop sound is audible
        rm.remove_mute();

        // Play audio feedback for recording stop
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string(); // Clone binding_id for the async task
        let cancel_generation = rm.cancel_generation();

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard {
                app: ah.clone(),
                operation: operation.clone(),
            };
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id, cancel_generation) {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                } else {
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let samples = Arc::new(samples);
                    let file_name = format!(
                        "murmur-{}.wav",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    );
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let mut pending_wav = match PendingWav::reserve(wav_path.clone()) {
                        Ok(wav) => Some(wav),
                        Err(error) => {
                            error!(
                                "Failed to reserve WAV path {}: {}",
                                wav_path.display(),
                                error
                            );
                            None
                        }
                    };
                    let wav_handle = pending_wav
                        .take()
                        .map(|wav| wav.write(Arc::clone(&samples)));

                    let provider = get_settings(&ah).transcription_provider;
                    let transcription_time = Instant::now();
                    let transcription_result = tm
                        .transcribe_with_provider(samples, operation.clone(), provider)
                        .await;

                    // Await WAV save and verify
                    let wav_saved = match wav_handle {
                        None => false,
                        Some(wav_handle) => match wav_handle.await {
                            Ok((wav, Ok(()))) => {
                                pending_wav = Some(wav);
                                match crate::audio_toolkit::verify_wav_file(&wav_path, sample_count)
                                {
                                    Ok(()) => true,
                                    Err(e) => {
                                        error!("WAV verification failed: {}", e);
                                        false
                                    }
                                }
                            }
                            Ok((wav, Err(e))) => {
                                pending_wav = Some(wav);
                                error!("Failed to save WAV file: {}", e);
                                false
                            }
                            Err(e) => {
                                error!("WAV save task panicked: {}", e);
                                false
                            }
                        },
                    };

                    if operation_was_cancelled(&operation, &rm, cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        return;
                    }

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?} ({} characters)",
                                transcription_time.elapsed(),
                                transcription.chars().count()
                            );

                            let Some(final_text) = complete_unless_cancelled(
                                process_transcription_output(&ah, &transcription, provider),
                                || operation_was_cancelled(&operation, &rm, cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                return;
                            };

                            if final_text.is_empty() {
                                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                    debug!("Transcription operation cancelled before history save");
                                    return;
                                }

                                if wav_saved {
                                    match hm.save_entry(file_name, final_text) {
                                        Ok(_) => {
                                            if let Some(wav) = pending_wav.as_mut() {
                                                wav.commit();
                                            }
                                        }
                                        Err(error) => {
                                            error!("Failed to save history entry: {}", error)
                                        }
                                    }
                                }
                            } else {
                                // `clipboard::paste` owns its native
                                // main-thread transactions and waits through
                                // the cancellable delayed auto-submit. Once
                                // injection has crossed its atomic boundary,
                                // a late cancellation can only suppress
                                // auto-submit; it must not discard history for
                                // text the user already received.
                                match crate::clipboard::paste(
                                    final_text.clone(),
                                    ah.clone(),
                                    operation.clone(),
                                )
                                .await
                                {
                                    Ok(outcome) => {
                                        if !paste_outcome_commits_history(outcome) {
                                            debug!("Transcription operation cancelled before paste execution");
                                            return;
                                        }
                                        if outcome == crate::clipboard::PasteOutcome::Skipped {
                                            error!("Transcription output could not be delivered");
                                            let _ = ah.emit("paste-error", ());
                                        }
                                        if wav_saved {
                                            match hm.save_entry(file_name, final_text) {
                                                Ok(_) => {
                                                    if let Some(wav) = pending_wav.as_mut() {
                                                        wav.commit();
                                                    }
                                                }
                                                Err(error) => {
                                                    error!(
                                                        "Failed to save history entry: {}",
                                                        error
                                                    )
                                                }
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        error!("Failed to paste transcription: {error}");
                                        let _ = ah.emit("paste-error", ());
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                return;
                            }

                            error!("Transcription failed: {}", err);
                            // Surface the failure to the UI (toast). The full
                            // message is also in murmur.log via the line above.
                            let _ = ah.emit("transcription-error", err.to_string());
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                    debug!("Transcription operation cancelled before failed history save");
                                    return;
                                }

                                match hm.save_entry(file_name, String::new()) {
                                    Ok(_) => {
                                        if let Some(wav) = pending_wav.as_mut() {
                                            wav.commit();
                                        }
                                    }
                                    Err(save_err) => {
                                        error!("Failed to save failed history entry: {}", save_err)
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.send_cancel();
        } else {
            utils::cancel_current_operation_before_coordinator(app);
        }
    }

    fn stop(
        &self,
        _app: &AppHandle,
        _binding_id: &str,
        _shortcut_str: &str,
        _operation: ProcessingOperation,
    ) {
    }
}

pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::{
        complete_unless_cancelled, output_language_for_provider, paste_outcome_commits_history,
        PendingWav,
    };
    use crate::clipboard::PasteOutcome;
    use crate::settings::TranscriptionProvider;
    use std::fs;
    use std::future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn completed_operation_returns_its_output() {
        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::ready("done"),
            || false,
        ));

        assert_eq!(result, Some("done"));
    }

    #[test]
    fn pending_operation_stops_after_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancelled_for_thread.store(true, Ordering::Release);
        });

        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::pending::<()>(),
            || cancelled.load(Ordering::Acquire),
        ));

        cancel_thread.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn uncommitted_wav_is_removed_when_its_owner_drops() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("pending.wav");

        drop(PendingWav::reserve(wav_path.clone()).unwrap());

        assert!(!wav_path.exists());
    }

    #[test]
    fn wav_writer_keeps_shared_samples_alive_and_cleans_uncommitted_output() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("shared-samples.wav");
        let samples = Arc::new(vec![0.25_f32, -0.5, 0.0]);

        let writer = PendingWav::reserve(wav_path.clone())
            .unwrap()
            .write(Arc::clone(&samples));
        drop(samples);

        let (wav, write_result) = tauri::async_runtime::block_on(writer).unwrap();
        write_result.unwrap();
        crate::audio_toolkit::verify_wav_file(&wav_path, 3).unwrap();

        drop(wav);
        assert!(!wav_path.exists());
    }

    #[test]
    fn committed_wav_survives_its_owner() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("committed.wav");

        let mut wav = PendingWav::reserve(wav_path.clone()).unwrap();
        fs::write(&wav_path, b"saved wav").unwrap();
        wav.commit();
        drop(wav);

        assert!(wav_path.exists());
    }

    #[test]
    fn uncommitted_wav_is_removed_during_panic_unwind() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("panic.wav");

        let result = std::panic::catch_unwind(|| {
            let _wav = PendingWav::reserve(wav_path.clone()).unwrap();
            panic!("simulated pipeline panic");
        });

        assert!(result.is_err());
        assert!(!wav_path.exists());
    }

    #[test]
    fn reserving_a_referenced_wav_never_claims_or_deletes_it() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("referenced.wav");
        fs::write(&wav_path, b"history-owned wav").unwrap();

        assert!(PendingWav::reserve(wav_path.clone()).is_err());

        assert_eq!(fs::read(&wav_path).unwrap(), b"history-owned wav");
    }

    #[test]
    fn gemini_ignores_a_preserved_codex_chinese_variant_preference() {
        assert_eq!(
            output_language_for_provider(TranscriptionProvider::Gemini, "zh-Hant"),
            None
        );
        assert_eq!(
            output_language_for_provider(TranscriptionProvider::Codex, "zh-Hant"),
            Some("zh-Hant")
        );
    }

    #[test]
    fn pasted_or_recoverable_output_survives_late_cancellation() {
        assert!(!paste_outcome_commits_history(PasteOutcome::Cancelled));
        assert!(paste_outcome_commits_history(PasteOutcome::Injected));
        assert!(paste_outcome_commits_history(PasteOutcome::IntentionalNone));
        assert!(paste_outcome_commits_history(PasteOutcome::Skipped));
    }
}
