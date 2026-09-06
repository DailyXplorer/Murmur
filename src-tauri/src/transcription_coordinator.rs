use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use crate::{OperationId, ProcessingOperation};
use log::{debug, error, warn};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);
const RELEASE_GRACE: Duration = Duration::from_millis(50);
const START_RETRY_DELAY: Duration = Duration::from_millis(25);
pub(crate) const QUIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const PROVIDER_QUIT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RECORDING_DURATION: Duration =
    Duration::from_secs(crate::audio_toolkit::constants::MAX_RECORDING_SECONDS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PttAction {
    Passthrough,
    DeferRelease,
    CancelRelease,
}

struct PendingRelease {
    binding_id: String,
    hotkey_string: String,
    deadline: Instant,
}

/// A press that arrived while the prior recording still owns the recorder.
/// Keep the user input and retry it after the trailing stop completes instead
/// of recording a phantom start or requiring another keypress.
struct PendingStart {
    binding_id: String,
    hotkey_string: String,
    deadline: Instant,
}

struct ActiveRecording {
    binding_id: String,
    hotkey_string: String,
    deadline: Instant,
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    Input {
        binding_id: String,
        hotkey_string: String,
        is_pressed: bool,
        push_to_talk: bool,
    },
    /// Local cancellation from a shortcut, the tray, or the webview.
    ///
    /// This must be processed here rather than at the source, because the
    /// coordinator's stage is authoritative while `stop_recording` runs on an
    /// async worker and the audio manager can still report recording active.
    Cancel,
    RemoteCancel,
    Shutdown,
    ProcessingFinished(OperationId),
}

/// Coordinates the short interval in which an irreversible Cmd+V has been
/// posted but its modifier-release receipt is still outstanding. It is
/// separate from provider state so quit can keep AppKit alive without waiting
/// on a network request or a provider mutex.
struct ShutdownDrain {
    closing: std::sync::atomic::AtomicBool,
    exit_allowed: std::sync::atomic::AtomicBool,
    finished: Mutex<bool>,
    finished_cv: Condvar,
}

impl ShutdownDrain {
    fn new() -> Self {
        Self {
            closing: std::sync::atomic::AtomicBool::new(false),
            exit_allowed: std::sync::atomic::AtomicBool::new(false),
            finished: Mutex::new(false),
            finished_cv: Condvar::new(),
        }
    }

    fn begin(&self) -> bool {
        !self.closing.swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    fn is_closing(&self) -> bool {
        self.closing.load(std::sync::atomic::Ordering::Acquire)
    }

    fn allow_exit(&self) -> bool {
        self.exit_allowed.load(std::sync::atomic::Ordering::Acquire)
    }

    fn finish(&self) {
        *self
            .finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        self.finished_cv.notify_all();
    }

    fn wait_bounded(&self, timeout: Duration) -> bool {
        let finished = self
            .finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (finished, _) = self
            .finished_cv
            .wait_timeout_while(finished, timeout, |finished| !*finished)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *finished
    }

    fn permit_exit(&self) {
        self.exit_allowed
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Pipeline lifecycle, owned exclusively by the coordinator thread.
enum Stage {
    Idle,
    Recording(ActiveRecording),
    Processing { operation: ProcessingOperation },
}

fn next_deadline(
    stage: &Stage,
    pending_release: Option<&PendingRelease>,
    pending_start: Option<&PendingStart>,
) -> Option<Instant> {
    let recording_deadline = match stage {
        Stage::Recording(recording) => Some(recording.deadline),
        _ => None,
    };
    [
        pending_release.map(|pending| pending.deadline),
        pending_start.map(|pending| pending.deadline),
        recording_deadline,
    ]
    .into_iter()
    .flatten()
    .min()
}

fn take_due_start(
    stage: &Stage,
    pending_start: &mut Option<PendingStart>,
    now: Instant,
) -> Option<(String, String)> {
    if !matches!(stage, Stage::Idle) {
        *pending_start = None;
        return None;
    }
    pending_start
        .as_ref()
        .is_some_and(|pending| pending.deadline <= now)
        .then(|| pending_start.take())
        .flatten()
        .map(|pending| (pending.binding_id, pending.hotkey_string))
}

fn take_due_stop(
    stage: &Stage,
    pending_release: &mut Option<PendingRelease>,
    now: Instant,
) -> Option<(String, String, bool)> {
    if pending_release
        .as_ref()
        .is_some_and(|pending| pending.deadline <= now)
    {
        if let Some(pending) = pending_release.take() {
            if matches!(stage, Stage::Recording(recording) if recording.binding_id == pending.binding_id)
            {
                return Some((pending.binding_id, pending.hotkey_string, false));
            }
        }
    }

    if let Stage::Recording(recording) = stage {
        if recording.deadline <= now {
            *pending_release = None;
            return Some((
                recording.binding_id.clone(),
                recording.hotkey_string.clone(),
                true,
            ));
        }
    }
    None
}

fn classify_ptt_event(
    pending_release_binding: Option<&str>,
    is_pressed: bool,
    push_to_talk: bool,
    binding_id: &str,
    recording_binding: Option<&str>,
) -> PttAction {
    if !push_to_talk {
        return PttAction::Passthrough;
    }

    if is_pressed {
        if pending_release_binding == Some(binding_id) {
            PttAction::CancelRelease
        } else {
            PttAction::Passthrough
        }
    } else if recording_binding == Some(binding_id) && pending_release_binding.is_none() {
        PttAction::DeferRelease
    } else {
        PttAction::Passthrough
    }
}

fn finish_cancel(
    stage: &mut Stage,
    pending_release: &mut Option<PendingRelease>,
    pending_start: &mut Option<PendingStart>,
) {
    *pending_release = None;
    *pending_start = None;
    *stage = Stage::Idle;
}

/// Returns whether cancellation may move the stage to Idle immediately.
/// After a paste claim, wait for the modifier-release receipt.
fn request_cancel(stage: &Stage) -> bool {
    match stage {
        Stage::Processing { operation } => operation.cancel(),
        Stage::Idle | Stage::Recording(_) => true,
    }
}

fn is_current_processing(stage: &Stage, id: OperationId) -> bool {
    matches!(stage, Stage::Processing { operation } if operation.id() == id)
}

fn queue_local_cancel(tx: &Sender<Command>) -> Result<(), mpsc::SendError<Command>> {
    tx.send(Command::Cancel)
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
    shutdown_drain: Arc<ShutdownDrain>,
}

/// Returns whether `id` names the sole supported transcription shortcut.
pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let shutdown_drain = Arc::new(ShutdownDrain::new());
        let worker_shutdown_drain = Arc::clone(&shutdown_drain);

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut next_operation_id = 1_u64;
                let mut last_press: Option<Instant> = None;
                let mut pending_release: Option<PendingRelease> = None;
                let mut pending_start: Option<PendingStart> = None;

                loop {
                    // Check deadlines before receiving so a continuously busy
                    // channel cannot postpone the hard recording limit.
                    if let Some((binding_id, hotkey_string, recording_limit)) =
                        take_due_stop(&stage, &mut pending_release, Instant::now())
                    {
                        if recording_limit {
                            warn!(
                                "Recording reached the 15-minute duration limit; stopping safely"
                            );
                        }
                        stop(
                            &app,
                            &mut stage,
                            &mut next_operation_id,
                            &binding_id,
                            &hotkey_string,
                        );
                        continue;
                    }

                    if let Some((binding_id, hotkey_string)) =
                        take_due_start(&stage, &mut pending_start, Instant::now())
                    {
                        if let Some(retry) = start(&app, &mut stage, &binding_id, &hotkey_string) {
                            pending_start = Some(retry);
                        }
                        continue;
                    }

                    let cmd = if let Some(deadline) =
                        next_deadline(&stage, pending_release.as_ref(), pending_start.as_ref())
                    {
                        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                            Ok(cmd) => cmd,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match rx.recv() {
                            Ok(cmd) => cmd,
                            Err(_) => break,
                        }
                    };

                    match cmd {
                        Command::Input {
                            binding_id,
                            hotkey_string,
                            is_pressed,
                            push_to_talk,
                        } => {
                            // A released push-to-talk key must not begin a
                            // recording after a deferred start finally finds
                            // the recorder idle.
                            if push_to_talk
                                && !is_pressed
                                && pending_start
                                    .as_ref()
                                    .is_some_and(|pending| pending.binding_id == binding_id)
                            {
                                pending_start = None;
                                continue;
                            }
                            let pending_release_binding = pending_release
                                .as_ref()
                                .map(|pending| pending.binding_id.as_str());
                            let recording_binding = match &stage {
                                Stage::Recording(recording) => Some(recording.binding_id.as_str()),
                                _ => None,
                            };

                            match classify_ptt_event(
                                pending_release_binding,
                                is_pressed,
                                push_to_talk,
                                &binding_id,
                                recording_binding,
                            ) {
                                PttAction::CancelRelease => {
                                    pending_release = None;
                                    continue;
                                }
                                PttAction::DeferRelease => {
                                    pending_release = Some(PendingRelease {
                                        binding_id,
                                        hotkey_string,
                                        deadline: Instant::now() + RELEASE_GRACE,
                                    });
                                    continue;
                                }
                                PttAction::Passthrough => {}
                            }

                            // Debounce rapid-fire press events (key repeat / double-tap).
                            // Push-to-talk releases may be deferred above to absorb X11 auto-repeat.
                            if is_pressed {
                                let now = Instant::now();
                                if last_press.is_some_and(|t| now.duration_since(t) < DEBOUNCE) {
                                    debug!("Debounced press for '{binding_id}'");
                                    continue;
                                }
                                last_press = Some(now);
                            }

                            if push_to_talk {
                                if is_pressed && matches!(stage, Stage::Idle) {
                                    pending_start =
                                        start(&app, &mut stage, &binding_id, &hotkey_string);
                                } else if !is_pressed
                                    && matches!(&stage, Stage::Recording(recording) if recording.binding_id == binding_id)
                                {
                                    stop(
                                        &app,
                                        &mut stage,
                                        &mut next_operation_id,
                                        &binding_id,
                                        &hotkey_string,
                                    );
                                }
                            } else if is_pressed {
                                match &stage {
                                    Stage::Idle => {
                                        pending_start =
                                            start(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    Stage::Recording(recording)
                                        if recording.binding_id == binding_id =>
                                    {
                                        stop(
                                            &app,
                                            &mut stage,
                                            &mut next_operation_id,
                                            &binding_id,
                                            &hotkey_string,
                                        );
                                    }
                                    _ => {
                                        debug!("Ignoring press for '{binding_id}': pipeline busy")
                                    }
                                }
                            }
                        }
                        Command::Cancel | Command::RemoteCancel => {
                            pending_release = None;
                            // Remote single-instance actions share this queue
                            // with remote toggles, and local cancellations must
                            // make the same decision from this authoritative
                            // stage. In particular, `stop_recording` moves us
                            // to Processing before its worker releases the
                            // audio manager, so that manager can still report
                            // recording active here.
                            let may_return_idle = request_cancel(&stage);
                            crate::utils::cancel_current_operation_from_coordinator(&app);
                            if may_return_idle {
                                finish_cancel(&mut stage, &mut pending_release, &mut pending_start);
                            } else {
                                // A Cmd+V was already posted. The async paste
                                // future will suppress delayed Enter and
                                // FinishGuard will transition to Idle after
                                // the modifier-release receipt.
                                pending_release = None;
                                pending_start = None;
                            }
                        }
                        Command::Shutdown => {
                            pending_release = None;
                            let may_return_idle = request_cancel(&stage);
                            crate::utils::cancel_current_operation_from_coordinator(&app);
                            if may_return_idle {
                                finish_cancel(&mut stage, &mut pending_release, &mut pending_start);
                                worker_shutdown_drain.finish();
                            } else {
                                pending_start = None;
                            }
                        }
                        Command::ProcessingFinished(id) => {
                            if is_current_processing(&stage, id) {
                                stage = Stage::Idle;
                                crate::shortcut::unregister_cancel_shortcut(&app);
                                crate::tray::change_tray_icon(
                                    &app,
                                    crate::tray::TrayIconState::Idle,
                                );
                                crate::utils::hide_recording_overlay(&app);
                                if worker_shutdown_drain.is_closing() {
                                    worker_shutdown_drain.finish();
                                }
                            } else {
                                debug!(
                                    "Ignoring stale transcription completion for operation {}",
                                    id.0
                                );
                            }
                        }
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self { tx, shutdown_drain }
    }

    /// Send a keyboard/signal input event for a transcribe binding.
    /// For signal-based toggles, use `is_pressed: true` and `push_to_talk: false`.
    pub fn send_input(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        push_to_talk: bool,
    ) {
        if self.shutdown_drain.is_closing() {
            debug!("Ignoring transcription input while the application is closing");
            return;
        }
        if self
            .tx
            .send(Command::Input {
                binding_id: binding_id.to_string(),
                hotkey_string: hotkey_string.to_string(),
                is_pressed,
                push_to_talk,
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Queue a local cancellation after any earlier lifecycle event.
    ///
    /// The coordinator supplies the authoritative stage to the cancellation
    /// helper, so it cannot mistake an asynchronously stopping recording for
    /// a fresh Recording stage.
    pub fn send_cancel(&self) {
        if queue_local_cancel(&self.tx).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Queues a remote cancellation behind remote transcription inputs. Unlike
    /// a local cancel shortcut, this must wait for an earlier remote toggle to
    /// reach the audio manager before it checks whether recording is active.
    pub fn send_remote_cancel(&self) {
        if self.tx.send(Command::RemoteCancel).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_processing_finished(&self, id: OperationId) {
        if self.tx.send(Command::ProcessingFinished(id)).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Starts a bounded foreground drain before application exit. The first
    /// request sends cancellation through the same serialized lifecycle; a
    /// second ExitRequested event is allowed only after the receipt arrives or
    /// the bounded timeout expires.
    pub fn begin_shutdown(&self, app: AppHandle) {
        if !self.shutdown_drain.begin() {
            return;
        }
        if self.tx.send(Command::Shutdown).is_err() {
            self.shutdown_drain.finish();
        }

        // Signal real transports before waiting for the last keyboard receipt.
        // This call only flips atomics, snapshots a short registry, and queues
        // Gemini cleanup off-main; it never acquires Gemini's runtime mutex.
        let provider_shutdown = app
            .try_state::<Arc<crate::managers::transcription::TranscriptionManager>>()
            .map(|manager| manager.begin_shutdown());

        let shutdown_drain = Arc::clone(&self.shutdown_drain);
        thread::spawn(move || {
            if !shutdown_drain.wait_bounded(QUIT_DRAIN_TIMEOUT) {
                warn!("Timed out waiting for the foreground paste release during shutdown");
            }
            if let Some(provider_shutdown) = provider_shutdown {
                if !provider_shutdown.wait_bounded(PROVIDER_QUIT_TIMEOUT) {
                    warn!("Timed out waiting for owned Gemini provider cleanup during shutdown");
                }
            }
            shutdown_drain.permit_exit();
            app.exit(0);
        });
    }

    pub fn allows_exit(&self) -> bool {
        self.shutdown_drain.allow_exit()
    }
}

fn start(
    app: &AppHandle,
    stage: &mut Stage,
    binding_id: &str,
    hotkey_string: &str,
) -> Option<PendingStart> {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return None;
    };
    action.start(app, binding_id, hotkey_string);
    let audio = app.try_state::<Arc<AudioRecordingManager>>()?;
    if audio.is_recording_started() {
        *stage = Stage::Recording(ActiveRecording {
            binding_id: binding_id.to_string(),
            hotkey_string: hotkey_string.to_string(),
            deadline: Instant::now() + MAX_RECORDING_DURATION,
        });
        None
    } else if audio.is_stopping() {
        debug!("Deferring start for '{binding_id}' until prior stop completes");
        Some(PendingStart {
            binding_id: binding_id.to_string(),
            hotkey_string: hotkey_string.to_string(),
            deadline: Instant::now() + START_RETRY_DELAY,
        })
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
        None
    }
}

fn stop(
    app: &AppHandle,
    stage: &mut Stage,
    next_operation_id: &mut u64,
    binding_id: &str,
    hotkey_string: &str,
) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    let operation = ProcessingOperation::new(OperationId(*next_operation_id));
    *next_operation_id = next_operation_id.wrapping_add(1);
    *stage = Stage::Processing {
        operation: operation.clone(),
    };
    action.stop(app, binding_id, hotkey_string, operation);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording_stage(deadline: Instant) -> Stage {
        Stage::Recording(ActiveRecording {
            binding_id: "transcribe".to_string(),
            hotkey_string: "CLI".to_string(),
            deadline,
        })
    }

    #[test]
    fn push_to_talk_release_while_recording_defers_release() {
        assert_eq!(
            classify_ptt_event(None, false, true, "transcribe", Some("transcribe")),
            PttAction::DeferRelease
        );
    }

    #[test]
    fn push_to_talk_press_matching_pending_release_cancels_release() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::CancelRelease
        );
    }

    #[test]
    fn toggle_mode_press_and_release_pass_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                false,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
        assert_eq!(
            classify_ptt_event(None, false, false, "transcribe", Some("transcribe")),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_for_different_binding_than_pending_release_passes_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "different_binding",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_matching_pending_release_cancels_without_recording_state() {
        assert_eq!(
            classify_ptt_event(Some("transcribe"), true, true, "transcribe", None),
            PttAction::CancelRelease
        );
    }

    #[test]
    fn remote_cancel_after_toggle_returns_the_coordinator_to_idle() {
        let mut stage = recording_stage(Instant::now() + MAX_RECORDING_DURATION);
        let mut pending_release = Some(PendingRelease {
            binding_id: "transcribe".to_string(),
            hotkey_string: "CLI".to_string(),
            deadline: Instant::now(),
        });

        finish_cancel(&mut stage, &mut pending_release, &mut None);

        assert!(matches!(stage, Stage::Idle));
        assert!(pending_release.is_none());
    }

    #[test]
    fn processing_cancel_returns_to_idle_while_audio_is_still_stopping() {
        let mut stage = Stage::Processing {
            operation: ProcessingOperation::new(OperationId(1)),
        };
        let mut pending_release = None;

        finish_cancel(&mut stage, &mut pending_release, &mut None);

        assert!(matches!(stage, Stage::Idle));
    }

    #[test]
    fn late_cancel_waits_for_paste_release_before_admitting_another_recording() {
        let operation = ProcessingOperation::new(OperationId(1));
        assert!(operation.try_enter_paste().is_some());
        let stage = Stage::Processing { operation };

        assert!(!request_cancel(&stage));
        assert!(matches!(stage, Stage::Processing { .. }));
    }

    #[test]
    fn quit_waits_for_a_fake_modifier_release_receipt() {
        let operation = ProcessingOperation::new(OperationId(1));
        assert!(operation.try_enter_paste().is_some());
        let stage = Stage::Processing { operation };
        assert!(!request_cancel(&stage));

        let drain = Arc::new(ShutdownDrain::new());
        assert!(drain.begin());
        let waiting_drain = Arc::clone(&drain);
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            tx.send(waiting_drain.wait_bounded(Duration::from_secs(5)))
                .unwrap();
        });

        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(10)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drain.finish();
        assert!(rx.recv().unwrap());
        waiter.join().unwrap();
    }

    #[test]
    fn local_cancel_during_processing_returns_to_idle_before_worker_finish() {
        let mut stage = Stage::Processing {
            operation: ProcessingOperation::new(OperationId(1)),
        };
        let mut pending_release = None;

        finish_cancel(&mut stage, &mut pending_release, &mut None);
        assert!(matches!(stage, Stage::Idle));
    }

    #[test]
    fn stale_completion_cannot_finish_a_later_operation() {
        let first = OperationId(1);
        let second = OperationId(2);
        let stage = Stage::Processing {
            operation: ProcessingOperation::new(second),
        };

        assert!(!is_current_processing(&stage, first));
        assert!(is_current_processing(&stage, second));
    }

    #[test]
    fn local_cancel_during_recording_cleans_up_immediately() {
        let mut stage = recording_stage(Instant::now() + MAX_RECORDING_DURATION);
        let mut pending_release = None;

        finish_cancel(&mut stage, &mut pending_release, &mut None);
        assert!(matches!(stage, Stage::Idle));
    }

    #[test]
    fn receiver_uses_the_earliest_release_or_recording_deadline() {
        let now = Instant::now();
        let stage = recording_stage(now + Duration::from_secs(10));
        let pending = PendingRelease {
            binding_id: "transcribe".to_string(),
            hotkey_string: "Option+Space".to_string(),
            deadline: now + Duration::from_millis(50),
        };
        assert_eq!(
            next_deadline(&stage, Some(&pending), None),
            Some(pending.deadline)
        );
        assert_eq!(
            next_deadline(&stage, None, None),
            Some(now + Duration::from_secs(10))
        );
    }

    #[test]
    fn deferred_start_waits_for_idle_and_never_creates_a_phantom_recording() {
        let now = Instant::now();
        let mut pending = Some(PendingStart {
            binding_id: "transcribe".to_string(),
            hotkey_string: "Option+Space".to_string(),
            deadline: now,
        });

        // Cancellation can leave the coordinator Idle while audio is Stopping.
        // Retrying remains a request until the recorder admits it.
        assert_eq!(
            take_due_start(&Stage::Idle, &mut pending, now),
            Some(("transcribe".to_string(), "Option+Space".to_string()))
        );
        assert!(pending.is_none());

        let mut stale = Some(PendingStart {
            binding_id: "transcribe".to_string(),
            hotkey_string: "Option+Space".to_string(),
            deadline: now,
        });
        assert_eq!(take_due_start(&recording_stage(now), &mut stale, now), None);
        assert!(stale.is_none());
    }

    #[test]
    fn expired_recording_is_taken_before_queued_input() {
        let now = Instant::now();
        let stage = recording_stage(now - Duration::from_millis(1));
        let mut pending_release = None;
        assert_eq!(
            take_due_stop(&stage, &mut pending_release, now),
            Some(("transcribe".to_string(), "CLI".to_string(), true))
        );
    }

    #[test]
    fn local_cancel_queues_once_without_waiting_for_the_coordinator() {
        let (tx, rx) = mpsc::channel();

        // `CancelAction` can call send_cancel from the coordinator thread
        // itself. An mpsc send adds one later command and never waits for that
        // thread to receive it, so there is neither recursive dispatch nor a
        // self-deadlock.
        assert!(queue_local_cancel(&tx).is_ok());
        assert!(matches!(rx.try_recv(), Ok(Command::Cancel)));
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    // ---------------------------------------------------------------------
    // Sequence-level regression coverage for issue #1539.
    //
    // Under X11 key auto-repeat, holding a push-to-talk key does not emit one
    // long press. It emits the initial press followed by a stream of
    // synthesized release/press pairs, then a single genuine release on key-up.
    // Before the fix, every synthesized release passed straight through and
    // stopped recording, so holding the key "rapidly toggled" recording on and
    // off. The fix defers each release for a short grace window and cancels it
    // when the matching auto-repeat press arrives.
    //
    // The unit tests above assert `classify_ptt_event` in isolation. The
    // simulator below threads that classifier through the same `pending_release`
    // / `stage` state transitions the coordinator loop performs (lines that
    // handle `Command::Input` and the `recv_timeout` grace expiry), so a whole
    // event burst can be exercised deterministically without a Tauri AppHandle
    // or real timers.
    // ---------------------------------------------------------------------

    const BINDING: &str = "transcribe";

    #[derive(Clone, Copy)]
    enum Ev {
        /// A key-down event (real initial press or a synthesized auto-repeat press).
        Press,
        /// A key-up event (synthesized auto-repeat release or the genuine key-up).
        Release,
        /// The `RELEASE_GRACE` window elapsed with no cancelling press arriving.
        Grace,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum SimStage {
        Idle,
        Recording,
        Processing,
    }

    struct SimResult {
        starts: u32,
        stops: u32,
        stage: SimStage,
    }

    /// Mirror of the coordinator loop's decision logic for a single push-to-talk
    /// binding: it calls the real `classify_ptt_event` and applies the exact same
    /// Defer / Cancel / debounce / start / stop transitions.
    fn simulate(events: &[Ev]) -> SimResult {
        let mut stage = SimStage::Idle;
        let mut pending: Option<String> = None;
        let mut last_press_ms: Option<u64> = None;
        let mut clock_ms: u64 = 0;
        let mut starts = 0u32;
        let mut stops = 0u32;
        let debounce_ms = DEBOUNCE.as_millis() as u64;

        for ev in events {
            // Auto-repeat events arrive a few ms apart, well inside DEBOUNCE.
            clock_ms += 5;

            match ev {
                Ev::Grace => {
                    // Coordinator's `RecvTimeoutError::Timeout` arm: fire the
                    // deferred release iff we are still recording that binding.
                    if let Some(pending_binding) = pending.take() {
                        if stage == SimStage::Recording && pending_binding == BINDING {
                            stage = SimStage::Processing;
                            stops += 1;
                        }
                    }
                }
                Ev::Press | Ev::Release => {
                    let is_pressed = matches!(ev, Ev::Press);
                    let pending_binding = pending.as_deref();
                    let recording_binding = if stage == SimStage::Recording {
                        Some(BINDING)
                    } else {
                        None
                    };

                    match classify_ptt_event(
                        pending_binding,
                        is_pressed,
                        true, // push_to_talk
                        BINDING,
                        recording_binding,
                    ) {
                        PttAction::CancelRelease => {
                            pending = None;
                            continue;
                        }
                        PttAction::DeferRelease => {
                            pending = Some(BINDING.to_string());
                            continue;
                        }
                        PttAction::Passthrough => {}
                    }

                    if is_pressed {
                        if last_press_ms.is_some_and(|t| clock_ms - t < debounce_ms) {
                            continue;
                        }
                        last_press_ms = Some(clock_ms);
                    }

                    if is_pressed && stage == SimStage::Idle {
                        stage = SimStage::Recording;
                        starts += 1;
                    } else if !is_pressed && stage == SimStage::Recording {
                        stage = SimStage::Processing;
                        stops += 1;
                    }
                }
            }
        }

        SimResult {
            starts,
            stops,
            stage,
        }
    }

    /// Initial press plus several synthesized release/press pairs, as X11 emits
    /// while a push-to-talk key is held down.
    fn autorepeat_burst() -> Vec<Ev> {
        let mut events = vec![Ev::Press];
        for _ in 0..6 {
            events.push(Ev::Release);
            events.push(Ev::Press);
        }
        events
    }

    /// Regression for #1539: a burst of X11 auto-repeat release/press pairs must
    /// not stop recording. Before the fix the first synthesized release stopped
    /// recording immediately (stops == 1, stage left Recording), which produced
    /// the rapid on/off toggling. With the fix the releases are coalesced and
    /// recording stays continuously active for the whole burst.
    #[test]
    fn x11_autorepeat_burst_does_not_toggle_recording() {
        let result = simulate(&autorepeat_burst());
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 0,
            "synthesized auto-repeat releases must not stop recording mid-burst"
        );
        assert_eq!(
            result.stage,
            SimStage::Recording,
            "recording must remain active across the entire auto-repeat burst"
        );
    }

    /// Complements the burst test: once the key is genuinely released and the
    /// grace window elapses with no re-press, recording stops exactly once. This
    /// proves the debounce only coalesces synthesized releases and does not wedge
    /// the coordinator or swallow the real key-up.
    #[test]
    fn genuine_release_after_grace_stops_recording_once() {
        let mut events = autorepeat_burst();
        events.push(Ev::Release); // genuine key-up
        events.push(Ev::Grace); // grace window elapses, no cancelling press
        let result = simulate(&events);
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 1,
            "a genuine release should stop recording exactly once"
        );
        assert_eq!(result.stage, SimStage::Processing);
    }
}
