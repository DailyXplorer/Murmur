use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::text_passes::{self, PassLanguage};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error, Capture};
use crate::cleanup::{CleanupOutcome, ProcessedText, PromptContext};
use crate::engine::EngineUsed;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::transcription::{Transcript, TranscriptionManager};
use crate::overlay::DictationEpoch;
use crate::recently_cancelled::{self, Kept};
use crate::settings::{get_settings, AppSettings, DictationStyle, OverlayStyle};
use crate::shortcut;
use crate::transcription_error::{FailureKind, TranscriptionError};
use crate::tray::{change_tray_icon, TrayIconState};
use crate::user_alert::{self, Alert};
use crate::utils::{self, show_mic_lost_notice, show_recording_overlay, show_transcribing_overlay};
use crate::window_names::{ContextReads, WindowContext};
use crate::{ProcessingOperation, TranscriptionCoordinator};
use log::{debug, error, info};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, Emitter};
use unicode_segmentation::UnicodeSegmentation;

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct TranscriptionErrorEvent {
    kind: FailureKind,
    detail: String,
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

/// Waits for the WAV writer and checks the file it left. `Some` hands back
/// the guard of a verified recording; anything else drops the guard, which
/// removes the file, and answers `None`.
async fn settle_wav(
    writer: Option<tauri::async_runtime::JoinHandle<(PendingWav, anyhow::Result<()>)>>,
    path: &Path,
    expected_samples: usize,
) -> Option<PendingWav> {
    match writer?.await {
        Ok((wav, Ok(()))) => match crate::audio_toolkit::verify_wav_file(path, expected_samples) {
            Ok(()) => Some(wav),
            Err(e) => {
                error!("WAV verification failed: {}", e);
                None
            }
        },
        Ok((_, Err(e))) => {
            error!("Failed to save WAV file: {}", e);
            None
        }
        Err(e) => {
            error!("WAV save task panicked: {}", e);
            None
        }
    }
}

/// Commits a history row. `recording` names the verified WAV, if any. A
/// transcript whose recording could not be saved is still kept, with an empty
/// file name, so it never depends on the WAV write.
fn save_history(
    hm: &HistoryManager,
    recording: Option<&str>,
    pending_wav: &mut Option<PendingWav>,
    text: ProcessedText,
    source: EngineUsed,
) {
    match hm.save_entry(recording.unwrap_or_default().to_string(), text, source) {
        Ok(_) => {
            if recording.is_some() {
                if let Some(wav) = pending_wav.as_mut() {
                    wav.commit();
                }
            }
        }
        Err(error) => error!("Failed to save history entry: {error}"),
    }
}

/// Converts the Chinese variant, then runs AI cleanup when it is on, a Codex
/// session exists and the dictation is long enough. `window` comes from the
/// window the user dictated into; `pass_language` is the language the text
/// passes ran in.
pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    pass_language: Option<PassLanguage>,
    window: WindowContext,
    operation: &ProcessingOperation,
) -> ProcessedText {
    process_transcription_output_in(app, transcription, pass_language, window, operation, None)
        .await
}

/// Like `process_transcription_output`, with `language` standing in for the
/// language setting when it is given: a re-transcription in a one-off language
/// converts the Chinese variant and hints the cleanup in that language.
pub(crate) async fn process_transcription_output_in(
    app: &AppHandle,
    transcription: &str,
    pass_language: Option<PassLanguage>,
    window: WindowContext,
    operation: &ProcessingOperation,
    language: Option<&str>,
) -> ProcessedText {
    let settings = crate::managers::transcription::with_language(get_settings(app), language);
    let final_text =
        crate::chinese_script::in_chosen_script(Some(&settings.selected_language), transcription);

    let available = settings.ai_cleanup_enabled && crate::cleanup::available();
    let plan = cleanup_plan(&settings, available, &final_text, window);
    deliver(
        final_text,
        plan,
        pass_language,
        |text, context| async move { crate::cleanup::clean(text, &context, operation).await },
    )
    .await
}

/// Below this many words, the default cleanup costs more wait than it adds.
const SHORT_DICTATION_WORDS: usize = 20;

#[derive(Debug)]
enum CleanupPlan {
    Off,
    Short { words: usize },
    Run(PromptContext),
}

/// Runs the plan. Text cleanup did not edit gets the spoken punctuation
/// pass; cleaned text already has its punctuation.
async fn deliver<F, Fut>(
    text: String,
    plan: CleanupPlan,
    pass_language: Option<PassLanguage>,
    clean: F,
) -> ProcessedText
where
    F: FnOnce(String, PromptContext) -> Fut,
    Fut: Future<Output = ProcessedText>,
{
    let processed = match plan {
        CleanupPlan::Off => ProcessedText::unprocessed(text),
        CleanupPlan::Short { words } => {
            info!("AI cleanup skipped: short dictation ({words} words)");
            ProcessedText::short_dictation(text)
        }
        CleanupPlan::Run(context) => clean(text, context).await,
    };
    match pass_language {
        Some(language) if processed.cleanup != Some(CleanupOutcome::Cleaned) => {
            let text = text_passes::raw_only(&processed.text, language);
            ProcessedText {
                raw: text.clone(),
                text,
                ..processed
            }
        }
        _ => processed,
    }
}

/// What a dictation starting now reads from the frontmost window: only
/// cleanup uses what it finds.
fn context_reads(settings: &AppSettings) -> ContextReads {
    let cleanup_reads = settings.window_names_enabled && settings.ai_cleanup_enabled;
    ContextReads {
        names: cleanup_reads,
        field: cleanup_reads,
    }
}

/// Whether and how to clean `text`. Only a plan that runs carries the
/// context, so nothing read from the window leaves the Mac otherwise.
///
/// A short dictation skips the default cleanup unless the user asked for it
/// (`cleanup_short_dictations`) or chose a transformation cleanup applies:
/// a Formal or Casual style. Vocabulary, window names, the field excerpt, the
/// language hint and whisper mode only help the default cleanup, so they do
/// not keep it.
fn cleanup_plan(
    settings: &AppSettings,
    available: bool,
    text: &str,
    window: WindowContext,
) -> CleanupPlan {
    if !(settings.ai_cleanup_enabled && available) {
        return CleanupPlan::Off;
    }
    let words = text.unicode_words().count();
    let explicit_transform = settings.dictation_style != DictationStyle::Normal;
    if (1..SHORT_DICTATION_WORDS).contains(&words)
        && !settings.cleanup_short_dictations
        && !explicit_transform
    {
        return CleanupPlan::Short { words };
    }
    CleanupPlan::Run(cleanup_context(settings, window))
}

fn cleanup_context(settings: &AppSettings, window: WindowContext) -> PromptContext {
    // The setting may have been turned off while the user was dictating.
    let window = if settings.window_names_enabled {
        window
    } else {
        WindowContext::default()
    };
    PromptContext {
        vocabulary: settings.custom_words.clone(),
        names: window.names,
        field: window.field,
        language: (settings.selected_language != "auto")
            .then(|| settings.selected_language.clone()),
        faint_speech: settings.whisper_mode,
        style: settings.dictation_style,
    }
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
                recently_cancelled::wipe(app);
                crate::window_names::capture(context_reads(&settings));
                let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
                // The connection warm-up first: it is quick, and the local
                // warm-up can block on a model load.
                std::thread::spawn(move || {
                    tm.warm_cloud_connections();
                    tm.warm_if_local();
                });
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
                    rm_clone.apply_mute(generation);
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
                let microphone_denied = is_microphone_access_denied(&err);
                let error_type = if microphone_denied {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                user_alert::show(
                    app,
                    if microphone_denied {
                        Alert::MicrophoneDenied
                    } else {
                        Alert::RecordingFailed
                    },
                );
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

        change_tray_icon(app, TrayIconState::Transcribing);
        show_transcribing_overlay(app);

        // Unmute before playing audio feedback so the stop sound is audible
        app.state::<Arc<AudioRecordingManager>>().remove_mute();

        play_feedback_sound(app, SoundType::Stop);

        Self::run_pipeline(
            app,
            operation,
            Input::Recording {
                binding_id: binding_id.to_string(),
            },
        );

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

/// What a dictation transcribes.
enum Input {
    /// The recording `stop` ends now.
    Recording { binding_id: String },
    /// A cancelled dictation the user restored.
    Restored(Kept),
}

/// Restore: runs a cancelled dictation through the pipeline as if its
/// recording had just ended. It pastes into the app focused now, so cleanup
/// reads that window too.
pub(crate) fn restore(app: &AppHandle, operation: ProcessingOperation, kept: Kept) {
    crate::window_names::capture(context_reads(&get_settings(app)));
    change_tray_icon(app, TrayIconState::Transcribing);
    show_transcribing_overlay(app);
    shortcut::register_cancel_shortcut(app);
    TranscribeAction::run_pipeline(app, operation, Input::Restored(kept));
}

/// The engine needs more than a second of audio, so a shorter capture is
/// padded with silence to 1.25 s. Returns the length captured: a cancel keeps
/// only that, so the 1 s minimum for Restore reads the real dictation.
fn pad_short_capture(mut samples: Vec<f32>) -> (Vec<f32>, usize) {
    let captured = samples.len();
    let second = TARGET_SAMPLE_RATE as usize;
    if captured > 0 && captured < second {
        samples.resize(second * 5 / 4, 0.0);
    }
    (samples, captured)
}

/// The audio a cancel keeps: what was captured, without the padding.
fn captured_audio(samples: Arc<Vec<f32>>, captured: usize) -> Vec<f32> {
    let mut samples = Arc::try_unwrap(samples).unwrap_or_else(|shared| shared.to_vec());
    samples.truncate(captured);
    samples
}

/// Keeps the audio of a dictation cancelled before its paste, for Restore.
/// `shown` is the overlay epoch of that dictation.
fn keep_cancelled(
    app: &AppHandle,
    samples: Arc<Vec<f32>>,
    captured: usize,
    transcript: Option<Transcript>,
    shown: DictationEpoch,
) {
    recently_cancelled::keep(app, captured_audio(samples, captured), transcript, shown);
}

impl TranscribeAction {
    fn run_pipeline(app: &AppHandle, operation: ProcessingOperation, input: Input) {
        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());
        let cancel_generation = rm.cancel_generation();
        // Its transcribing state showed just before: a newer dictation takes
        // the overlay after it, so a pill raised for this one by a late cancel
        // can tell them apart.
        let shown = utils::dictation_epoch();

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard {
                app: ah.clone(),
                operation: operation.clone(),
            };
            debug!("Starting async transcription task");

            let stop_recording_time = Instant::now();
            let window = crate::window_names::collect();
            let captured = match input {
                Input::Recording { binding_id } => rm
                    .stop_recording(&binding_id, cancel_generation)
                    .map(|capture| (capture, None)),
                Input::Restored(mut kept) => {
                    let samples = std::mem::take(&mut kept.samples);
                    let transcript = kept.transcript.take();
                    Some((
                        Capture {
                            samples,
                            mic_lost: false,
                        },
                        transcript,
                    ))
                }
            };
            if let Some((Capture { samples, mic_lost }, kept_transcript)) = captured {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    recently_cancelled::keep(&ah, samples, kept_transcript, shown);
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    if mic_lost {
                        show_mic_lost_notice(&ah, &operation);
                    }
                } else {
                    let (samples, captured) = pad_short_capture(samples);
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let samples = Arc::new(samples);
                    let file_name = format!(
                        "murmur-{}.wav",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    );
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let pending_wav = match PendingWav::reserve(wav_path.clone()) {
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
                    let wav_handle = pending_wav.map(|wav| wav.write(Arc::clone(&samples)));

                    if crate::codex_transcribe::upload_size_for_samples(sample_count)
                        > crate::codex_transcribe::UPLOAD_SOFT_LIMIT_BYTES
                    {
                        user_alert::show(&ah, Alert::LongRecording);
                    }
                    let transcription_time = Instant::now();
                    // A dictation cancelled after the engine answered is
                    // restored from its transcript.
                    let transcription_result = match kept_transcript {
                        Some(transcript) => Ok(transcript),
                        None => {
                            tm.transcribe_samples(Arc::clone(&samples), operation.clone())
                                .await
                        }
                    };

                    if operation_was_cancelled(&operation, &rm, cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        keep_cancelled(&ah, samples, captured, transcription_result.ok(), shown);
                        return;
                    }

                    match transcription_result {
                        Ok(crate::managers::transcription::Transcript {
                            text: transcription,
                            dictionary_fixes,
                            source,
                            pass_language,
                        }) => {
                            debug!(
                                "Transcription completed in {:?} ({} characters)",
                                transcription_time.elapsed(),
                                transcription.chars().count()
                            );

                            let Some(processed) = complete_unless_cancelled(
                                process_transcription_output(
                                    &ah,
                                    &transcription,
                                    pass_language,
                                    window,
                                    &operation,
                                ),
                                || operation_was_cancelled(&operation, &rm, cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                keep_cancelled(
                                    &ah,
                                    samples,
                                    captured,
                                    Some(Transcript {
                                        text: transcription,
                                        dictionary_fixes,
                                        source,
                                        pass_language,
                                    }),
                                    shown,
                                );
                                return;
                            };
                            let final_text = processed.text.clone();

                            if final_text.is_empty() {
                                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                    debug!("Transcription operation cancelled before history save");
                                    keep_cancelled(
                                        &ah,
                                        samples,
                                        captured,
                                        Some(Transcript {
                                            text: transcription,
                                            dictionary_fixes,
                                            source,
                                            pass_language,
                                        }),
                                        shown,
                                    );
                                    return;
                                }

                                let mut pending_wav =
                                    settle_wav(wav_handle, &wav_path, sample_count).await;
                                if pending_wav.is_some() {
                                    save_history(
                                        &hm,
                                        Some(file_name.as_str()),
                                        &mut pending_wav,
                                        processed,
                                        source,
                                    );
                                }
                                // A cancel during the save leaves the
                                // overlay to the dictation after it.
                                if mic_lost {
                                    show_mic_lost_notice(&ah, &operation);
                                }
                            } else {
                                // `clipboard::paste` owns its native
                                // main-thread transactions and waits through
                                // the cancellable delayed auto-submit. Once
                                // injection has crossed its atomic boundary,
                                // a late cancellation can only suppress
                                // auto-submit; it must not discard history for
                                // text the user already received.
                                let delivered = match crate::clipboard::paste(
                                    final_text.clone(),
                                    ah.clone(),
                                    operation.clone(),
                                    mic_lost,
                                )
                                .await
                                {
                                    Ok(outcome) if !paste_outcome_commits_history(outcome) => {
                                        debug!("Transcription operation cancelled before paste execution");
                                        keep_cancelled(
                                            &ah,
                                            samples,
                                            captured,
                                            Some(Transcript {
                                                text: transcription,
                                                dictionary_fixes,
                                                source,
                                                pass_language,
                                            }),
                                            shown,
                                        );
                                        return;
                                    }
                                    Ok(outcome) => {
                                        // The warning follows the copy pill
                                        // (`Notice::Copy { mic_lost_next }`),
                                        // and a failed paste shows its error.
                                        if mic_lost
                                            && matches!(
                                                outcome,
                                                crate::clipboard::PasteOutcome::Injected
                                                    | crate::clipboard::PasteOutcome::IntentionalNone
                                            )
                                        {
                                            show_mic_lost_notice(&ah, &operation);
                                        }
                                        outcome != crate::clipboard::PasteOutcome::Skipped
                                    }
                                    Err(error) => {
                                        error!("Failed to paste transcription: {error}");
                                        false
                                    }
                                };
                                // Only a delivered dictation counts: the
                                // frontmost app is then the one that got it.
                                if delivered {
                                    crate::insights::record_dictation(
                                        &ah,
                                        &final_text,
                                        sample_count,
                                        dictionary_fixes,
                                    );
                                }
                                // The recording is settled only now, after the
                                // paste: the text never waits for the WAV
                                // writer or its check. The transcript is kept
                                // whatever happened to the paste or the WAV:
                                // history first, then the clipboard as the
                                // immediate way back.
                                let mut pending_wav =
                                    settle_wav(wav_handle, &wav_path, sample_count).await;
                                let recording = pending_wav.is_some().then_some(file_name.as_str());
                                save_history(&hm, recording, &mut pending_wav, processed, source);
                                if !delivered {
                                    error!("Transcription output could not be delivered");
                                    crate::clipboard::copy_for_recovery(&ah, final_text);
                                    let _ = ah.emit("paste-error", ());
                                    user_alert::show(&ah, Alert::PasteFailed);
                                }
                            }
                        }
                        Err(err) => {
                            if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                keep_cancelled(&ah, samples, captured, None, shown);
                                return;
                            }

                            error!("Transcription failed: {:#}", err);
                            let kind = TranscriptionError::kind_of(&err);
                            let _ = ah.emit(
                                "transcription-error",
                                TranscriptionErrorEvent {
                                    kind,
                                    detail: format!("{err:#}"),
                                },
                            );
                            user_alert::show(&ah, Alert::Transcription(kind));
                            // Save entry with empty text so user can retry
                            let mut pending_wav =
                                settle_wav(wav_handle, &wav_path, sample_count).await;
                            if pending_wav.is_some() {
                                if operation_was_cancelled(&operation, &rm, cancel_generation) {
                                    debug!("Transcription operation cancelled before failed history save");
                                    keep_cancelled(&ah, samples, captured, None, shown);
                                    return;
                                }
                                // Nothing transcribed it; Retry records the
                                // engine that does.
                                save_history(
                                    &hm,
                                    Some(file_name.as_str()),
                                    &mut pending_wav,
                                    ProcessedText::unprocessed(String::new()),
                                    EngineUsed {
                                        engine: get_settings(&ah).transcription_engine,
                                        fallback: None,
                                    },
                                );
                            }
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
            }
        });
    }
}

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
    map.insert(
        crate::paste_last::BINDING_ID.to_string(),
        Arc::new(crate::paste_last::PasteLastAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        crate::meetings::BINDING_ID.to_string(),
        Arc::new(crate::meetings::MeetingToggleAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        crate::meetings::live::PASTE_BINDING_ID.to_string(),
        Arc::new(crate::meetings::live::paste::PasteNewAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::{
        captured_audio, cleanup_context, cleanup_plan, complete_unless_cancelled, context_reads,
        deliver, pad_short_capture, paste_outcome_commits_history, settle_wav, CleanupPlan,
        PendingWav, SHORT_DICTATION_WORDS,
    };
    use crate::audio_toolkit::text_passes::PassLanguage;
    use crate::cleanup::{CleanupOutcome, ProcessedText, PromptContext};
    use crate::clipboard::PasteOutcome;
    use crate::field_excerpt::FieldExcerpt;
    use crate::settings::{get_default_settings, AppSettings, DictationStyle};
    use crate::window_names::{ContextReads, WindowContext};
    use std::cell::Cell;
    use std::fs;
    use std::future;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn a_cancel_keeps_the_captured_audio_not_the_padding() {
        let (padded, captured) = pad_short_capture(vec![0.5; 8_000]);
        assert_eq!(padded.len(), 20_000);
        let kept = captured_audio(Arc::new(padded), captured);
        assert_eq!(kept, vec![0.5; 8_000]);
        assert!(
            kept.len() < crate::recently_cancelled::MIN_SAMPLES,
            "a half-second dictation is too short for Restore, padded or not"
        );

        let (samples, captured) = pad_short_capture(vec![0.5; 24_000]);
        assert_eq!(samples.len(), 24_000);
        assert_eq!(captured_audio(Arc::new(samples), captured).len(), 24_000);
    }

    #[test]
    fn cleanup_follows_the_chosen_dictation_style() {
        for style in [
            DictationStyle::Formal,
            DictationStyle::Normal,
            DictationStyle::Casual,
        ] {
            let mut settings = get_default_settings();
            settings.dictation_style = style;
            assert_eq!(
                cleanup_context(&settings, WindowContext::default()).style,
                style
            );
        }
    }

    #[test]
    fn the_window_is_read_only_with_cleanup_and_the_setting_on() {
        for (setting, cleanup, read) in [
            (true, true, true),
            (false, true, false),
            (true, false, false),
            (false, false, false),
        ] {
            let mut settings = get_default_settings();
            settings.window_names_enabled = setting;
            settings.ai_cleanup_enabled = cleanup;
            assert_eq!(
                context_reads(&settings),
                ContextReads {
                    names: read,
                    field: read
                },
                "setting {setting}, cleanup {cleanup}"
            );
        }
    }

    fn window() -> WindowContext {
        WindowContext {
            names: vec!["Ingrid".to_string()],
            field: Some(FieldExcerpt {
                before: "Hi Ingrid, ".to_string(),
                after: String::new(),
            }),
        }
    }

    const LONG: &str = "I wanted to let you know that the report is finished and that I \
        will send it to the whole team before the meeting tomorrow morning";
    const SHORT: &str = "send it to the team today";

    fn request(
        settings: &AppSettings,
        available: bool,
        window: WindowContext,
    ) -> Option<PromptContext> {
        match cleanup_plan(settings, available, LONG, window) {
            CleanupPlan::Run(context) => Some(context),
            _ => None,
        }
    }

    /// Runs `text` through the output step with a fake cleanup that replies
    /// `reply`, or fails without one. Returns the result and whether cleanup
    /// was asked.
    fn deliver_with(
        settings: &AppSettings,
        text: &str,
        language: Option<PassLanguage>,
        reply: Option<&str>,
    ) -> (ProcessedText, bool) {
        let asked = Cell::new(false);
        let plan = cleanup_plan(settings, true, text, WindowContext::default());
        let processed = tauri::async_runtime::block_on(deliver(
            text.to_string(),
            plan,
            language,
            |raw, _context| {
                asked.set(true);
                let processed = match reply {
                    Some(reply) => ProcessedText {
                        text: reply.to_string(),
                        raw,
                        cleanup: Some(CleanupOutcome::Cleaned),
                    },
                    None => ProcessedText {
                        text: raw.clone(),
                        raw,
                        cleanup: Some(CleanupOutcome::Failed),
                    },
                };
                future::ready(processed)
            },
        ));
        (processed, asked.get())
    }

    fn cleanup_on() -> AppSettings {
        let mut settings = get_default_settings();
        settings.ai_cleanup_enabled = true;
        settings
    }

    #[test]
    fn short_dictation_skips_default_cleanup() {
        let (processed, asked) = deliver_with(&cleanup_on(), SHORT, None, Some("Cleaned."));
        assert!(!asked);
        assert_eq!(processed, ProcessedText::short_dictation(SHORT.to_string()));
    }

    #[test]
    fn short_dictation_keeps_explicit_transform() {
        for style in [DictationStyle::Formal, DictationStyle::Casual] {
            let mut settings = cleanup_on();
            settings.dictation_style = style;
            let (processed, asked) = deliver_with(&settings, SHORT, None, Some("Cleaned."));
            assert!(asked, "{style:?}");
            assert_eq!(processed.cleanup, Some(CleanupOutcome::Cleaned));
        }
    }

    #[test]
    fn long_dictation_still_cleaned() {
        assert!(LONG.unicode_words().count() >= SHORT_DICTATION_WORDS);
        let (processed, asked) = deliver_with(&cleanup_on(), LONG, None, Some("Cleaned."));
        assert!(asked);
        assert_eq!(processed.text, "Cleaned.");
    }

    #[test]
    fn cjk_text_is_counted_by_words() {
        let long = "我们明天上午十点在会议室开会讨论新产品的发布计划和预算";
        assert_eq!(long.split_whitespace().count(), 1);
        let (_, asked) = deliver_with(&cleanup_on(), long, None, Some("Cleaned."));
        assert!(asked, "{} words", long.unicode_words().count());

        let short = "我们明天开会";
        let (processed, asked) = deliver_with(&cleanup_on(), short, None, Some("Cleaned."));
        assert!(!asked);
        assert_eq!(processed.cleanup, Some(CleanupOutcome::ShortDictation));
    }

    #[test]
    fn setting_on_cleans_short_dictations() {
        let mut settings = cleanup_on();
        settings.cleanup_short_dictations = true;
        let (processed, asked) = deliver_with(&settings, SHORT, None, Some("Cleaned."));
        assert!(asked);
        assert_eq!(processed.cleanup, Some(CleanupOutcome::Cleaned));
    }

    #[test]
    fn raw_fallback_gets_spoken_punctuation() {
        let english = Some(PassLanguage::En);
        let mut off = cleanup_on();
        off.ai_cleanup_enabled = false;
        let (processed, _) = deliver_with(&off, "hello comma world", english, None);
        assert_eq!(
            processed,
            ProcessedText::unprocessed("hello, world".to_string())
        );

        let (processed, _) = deliver_with(&cleanup_on(), "hello comma world", english, None);
        assert_eq!(
            processed,
            ProcessedText::short_dictation("hello, world".to_string())
        );

        let failed = format!("{LONG} comma thanks");
        let (processed, asked) = deliver_with(&cleanup_on(), &failed, english, None);
        assert!(asked);
        assert_eq!(processed.text, format!("{LONG}, thanks"));
        assert_eq!(processed.cleanup, Some(CleanupOutcome::Failed));
    }

    #[test]
    fn cleaned_text_is_not_punctuated_twice() {
        let (processed, _) = deliver_with(
            &cleanup_on(),
            LONG,
            Some(PassLanguage::En),
            Some("Say comma twice"),
        );
        assert_eq!(processed.text, "Say comma twice");
    }

    #[test]
    fn nothing_is_sent_with_cleanup_off() {
        let mut settings = get_default_settings();
        settings.window_names_enabled = true;
        settings.ai_cleanup_enabled = false;
        assert!(request(&settings, true, window()).is_none());
    }

    #[test]
    fn nothing_is_sent_without_a_session() {
        let mut settings = get_default_settings();
        settings.window_names_enabled = true;
        settings.ai_cleanup_enabled = true;
        assert!(request(&settings, false, window()).is_none());
    }

    #[test]
    fn a_setting_turned_off_while_dictating_drops_names_and_excerpt() {
        let mut settings = get_default_settings();
        settings.window_names_enabled = true;
        settings.ai_cleanup_enabled = true;
        settings.custom_words = vec!["Murmur".to_string()];
        let context = request(&settings, true, window()).expect("cleanup runs");
        assert_eq!(context.names, vec!["Ingrid"]);
        assert_eq!(context.field, window().field);

        settings.window_names_enabled = false;
        let context = request(&settings, true, window()).expect("cleanup runs");
        assert!(context.names.is_empty());
        assert_eq!(context.field, None);
        assert_eq!(context.vocabulary, vec!["Murmur"]);
    }

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
    fn settle_wav_hands_back_a_verified_recording() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("verified.wav");
        let writer = PendingWav::reserve(wav_path.clone())
            .unwrap()
            .write(Arc::new(vec![0.1_f32; 160]));

        let settled = tauri::async_runtime::block_on(settle_wav(Some(writer), &wav_path, 160));

        let mut wav = settled.expect("a complete file is kept");
        assert!(wav_path.exists());
        wav.commit();
    }

    #[test]
    fn settle_wav_removes_a_file_that_fails_its_check() {
        let directory = tempfile::tempdir().unwrap();
        let wav_path = directory.path().join("short.wav");
        let writer = PendingWav::reserve(wav_path.clone())
            .unwrap()
            .write(Arc::new(vec![0.1_f32; 160]));

        let settled = tauri::async_runtime::block_on(settle_wav(Some(writer), &wav_path, 161));

        assert!(settled.is_none());
        assert!(!wav_path.exists());
    }

    #[test]
    fn settle_wav_without_a_writer_has_nothing_to_keep() {
        let path = Path::new("never-written.wav");
        assert!(tauri::async_runtime::block_on(settle_wav(None, path, 0)).is_none());
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
    fn pasted_or_recoverable_output_survives_late_cancellation() {
        assert!(!paste_outcome_commits_history(PasteOutcome::Cancelled));
        assert!(paste_outcome_commits_history(PasteOutcome::Injected));
        assert!(paste_outcome_commits_history(PasteOutcome::CopyOffered));
        assert!(paste_outcome_commits_history(PasteOutcome::IntentionalNone));
        assert!(paste_outcome_commits_history(PasteOutcome::Skipped));
    }
}
