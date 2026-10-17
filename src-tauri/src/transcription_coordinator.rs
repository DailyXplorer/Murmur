use crate::actions::ACTION_MAP;
use crate::dictation_gesture::{Gesture, Intent, Key};
use crate::managers::audio::AudioRecordingManager;
use crate::{OperationId, ProcessingOperation};
use log::{debug, error, warn};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);
const START_RETRY_DELAY: Duration = Duration::from_millis(25);
pub(crate) const QUIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const PROVIDER_QUIT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RECORDING_DURATION: Duration =
    Duration::from_secs(crate::audio_toolkit::constants::MAX_RECORDING_SECONDS);
/// How long before the recording cap the user is told it will stop.
pub(crate) const LIMIT_WARNING_LEAD: Duration = Duration::from_secs(60);

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
    limit_warned: bool,
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    /// A press or release of the recording shortcut, stamped when it fired so
    /// a busy queue cannot stretch a tap into a hold.
    Key {
        binding_id: String,
        hotkey_string: String,
        key: Key,
        at: Instant,
    },
    /// A remote toggle from the CLI or SIGUSR2.
    Toggle {
        binding_id: String,
        hotkey_string: String,
    },
    /// Local cancellation from a shortcut, the tray, or the webview.
    ///
    /// This must be processed here rather than at the source, because the
    /// coordinator's stage is authoritative while `stop_recording` runs on an
    /// async worker and the audio manager can still report recording active.
    Cancel,
    RemoteCancel,
    /// Paste the latest transcription again. Admitted only while idle, so it
    /// never interleaves with a dictation's own paste.
    PasteLast,
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

fn limit_warning_at(recording: &ActiveRecording) -> Instant {
    recording
        .deadline
        .checked_sub(LIMIT_WARNING_LEAD)
        .unwrap_or(recording.deadline)
}

fn next_deadline(
    stage: &Stage,
    gesture: Gesture,
    pending_start: Option<&PendingStart>,
) -> Option<Instant> {
    let recording_deadline = match stage {
        Stage::Recording(recording) if recording.limit_warned => Some(recording.deadline),
        Stage::Recording(recording) => Some(limit_warning_at(recording)),
        _ => None,
    };
    [
        gesture.deadline(),
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

fn take_due_limit_stop(stage: &Stage, now: Instant) -> Option<(String, String)> {
    match stage {
        Stage::Recording(recording) if recording.deadline <= now => Some((
            recording.binding_id.clone(),
            recording.hotkey_string.clone(),
        )),
        _ => None,
    }
}

/// Returns true once per recording, when the cap is `LIMIT_WARNING_LEAD` away.
fn take_due_limit_warning(stage: &mut Stage, now: Instant) -> bool {
    match stage {
        Stage::Recording(recording)
            if !recording.limit_warned && limit_warning_at(recording) <= now =>
        {
            recording.limit_warned = true;
            true
        }
        _ => false,
    }
}

/// What the coordinator handles next, once the recording cap is checked.
enum Next {
    Command(Command),
    /// The double-tap window closed with no second press queued.
    GestureExpired(Gesture),
    /// Nothing is queued and no gesture deadline has passed.
    Wait,
    Closed,
}

/// Reads a queued command before it judges the gesture deadline. Key events
/// carry the time they fired, so a second tap that queued while the coordinator
/// was busy (for example opening a slow microphone) still lands inside the
/// window. Only an empty queue proves that no second press came in time.
fn next_step(rx: &Receiver<Command>, gesture: Gesture, now: Instant) -> Next {
    match rx.try_recv() {
        Ok(cmd) => Next::Command(cmd),
        Err(mpsc::TryRecvError::Disconnected) => Next::Closed,
        Err(mpsc::TryRecvError::Empty) => match gesture.on_deadline(now) {
            (expired, Intent::Discard) => Next::GestureExpired(expired),
            _ => Next::Wait,
        },
    }
}

fn is_bounce(last_press: Option<Instant>, at: Instant) -> bool {
    last_press.is_some_and(|previous| at.saturating_duration_since(previous) < DEBOUNCE)
}

/// A gesture only means something while its recording exists or is queued.
/// Otherwise (busy pipeline, microphone failure, finished recording) the next
/// press must start afresh rather than stop a recording that is not there.
fn gesture_has_recording(stage: &Stage, pending_start: Option<&PendingStart>) -> bool {
    matches!(stage, Stage::Recording(_)) || pending_start.is_some()
}

fn finish_cancel(stage: &mut Stage, pending_start: &mut Option<PendingStart>) {
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

/// State owned by the coordinator thread.
struct Worker {
    app: AppHandle,
    shutdown_drain: Arc<ShutdownDrain>,
    stage: Stage,
    gesture: Gesture,
    pending_start: Option<PendingStart>,
    next_operation_id: u64,
    last_press: Option<Instant>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        loop {
            // Check deadlines before receiving so a continuously busy
            // channel cannot postpone the hard recording limit.
            let now = Instant::now();
            if let Some((binding_id, hotkey_string)) = take_due_limit_stop(&self.stage, now) {
                warn!("Recording reached the 15-minute duration limit; stopping safely");
                self.gesture = Gesture::Idle;
                self.stop(&binding_id, &hotkey_string);
                continue;
            }

            if take_due_limit_warning(&mut self.stage, now) {
                crate::user_alert::show(&self.app, crate::user_alert::Alert::RecordingLimitSoon);
                continue;
            }

            let cmd = match next_step(&rx, self.gesture, now) {
                Next::Command(cmd) => cmd,
                Next::GestureExpired(gesture) => {
                    self.gesture = gesture;
                    self.discard();
                    continue;
                }
                Next::Closed => break,
                Next::Wait => {
                    if let Some((binding_id, hotkey_string)) =
                        take_due_start(&self.stage, &mut self.pending_start, now)
                    {
                        self.pending_start =
                            start(&self.app, &mut self.stage, &binding_id, &hotkey_string);
                        self.forget_gesture_without_recording();
                        continue;
                    }

                    // Wake at the next deadline and come back through the
                    // queue check above rather than judging it here.
                    match next_deadline(&self.stage, self.gesture, self.pending_start.as_ref()) {
                        Some(deadline) => match rx
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        {
                            Ok(cmd) => cmd,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        },
                        None => match rx.recv() {
                            Ok(cmd) => cmd,
                            Err(_) => break,
                        },
                    }
                }
            };
            self.handle(cmd);
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Key {
                binding_id,
                hotkey_string,
                key,
                at,
            } => {
                if key == Key::Press {
                    if is_bounce(self.last_press, at) {
                        debug!("Debounced press for '{binding_id}'");
                        return;
                    }
                    self.last_press = Some(at);
                }
                let (gesture, intent) = self.gesture.on_key(key, at);
                self.gesture = gesture;
                self.apply(intent, &binding_id, &hotkey_string);
            }
            Command::Toggle {
                binding_id,
                hotkey_string,
            } => {
                let (gesture, intent) = self.gesture.on_toggle();
                self.gesture = gesture;
                self.apply(intent, &binding_id, &hotkey_string);
            }
            Command::Cancel | Command::RemoteCancel => {
                self.gesture = Gesture::Idle;
                // Remote single-instance actions share this queue with
                // remote toggles, and local cancellations must make the
                // same decision from this authoritative stage. In
                // particular, `stop_recording` moves us to Processing
                // before its worker releases the audio manager, so that
                // manager can still report recording active here.
                let may_return_idle = request_cancel(&self.stage);
                crate::utils::cancel_current_operation_from_coordinator(&self.app);
                if may_return_idle {
                    finish_cancel(&mut self.stage, &mut self.pending_start);
                } else {
                    // A Cmd+V was already posted. The async paste future
                    // will suppress delayed Enter and FinishGuard will
                    // transition to Idle after the modifier-release
                    // receipt.
                    self.pending_start = None;
                }
            }
            Command::PasteLast => {
                if matches!(self.stage, Stage::Idle) {
                    let operation = self.next_operation();
                    self.stage = Stage::Processing {
                        operation: operation.clone(),
                    };
                    crate::paste_last::spawn(&self.app, operation);
                } else {
                    debug!("Ignoring paste-last request: dictation in progress");
                }
            }
            Command::Shutdown => {
                self.gesture = Gesture::Idle;
                let may_return_idle = request_cancel(&self.stage);
                crate::utils::cancel_current_operation_from_coordinator(&self.app);
                if may_return_idle {
                    finish_cancel(&mut self.stage, &mut self.pending_start);
                    self.shutdown_drain.finish();
                } else {
                    self.pending_start = None;
                }
            }
            Command::ProcessingFinished(id) => {
                if is_current_processing(&self.stage, id) {
                    self.stage = Stage::Idle;
                    crate::shortcut::unregister_cancel_shortcut(&self.app);
                    crate::tray::change_tray_icon(&self.app, crate::tray::TrayIconState::Idle);
                    crate::utils::hide_recording_overlay(&self.app);
                    if self.shutdown_drain.is_closing() {
                        self.shutdown_drain.finish();
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

    fn apply(&mut self, intent: Intent, binding_id: &str, hotkey_string: &str) {
        match intent {
            Intent::None | Intent::LockHandsFree => {}
            Intent::Start | Intent::StartHandsFree => {
                if matches!(self.stage, Stage::Idle) {
                    self.pending_start =
                        start(&self.app, &mut self.stage, binding_id, hotkey_string);
                } else {
                    debug!("Ignoring press for '{binding_id}': pipeline busy");
                }
            }
            Intent::Stop => match &self.stage {
                Stage::Recording(recording) if recording.binding_id == binding_id => {
                    self.stop(binding_id, hotkey_string);
                }
                _ => self.pending_start = None,
            },
            Intent::Discard => self.discard(),
            Intent::Restart => {
                self.discard();
                self.apply(Intent::Start, binding_id, hotkey_string);
            }
        }
        self.forget_gesture_without_recording();
    }

    /// Drops a lone short tap's recording without uploading it.
    fn discard(&mut self) {
        self.pending_start = None;
        if matches!(self.stage, Stage::Recording(_)) {
            debug!("Discarding a single short tap");
            crate::utils::cancel_current_operation_from_coordinator(&self.app);
            self.stage = Stage::Idle;
        }
    }

    fn forget_gesture_without_recording(&mut self) {
        if !gesture_has_recording(&self.stage, self.pending_start.as_ref()) {
            self.gesture = Gesture::Idle;
        }
    }

    fn next_operation(&mut self) -> ProcessingOperation {
        let operation = ProcessingOperation::new(OperationId(self.next_operation_id));
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        operation
    }

    fn stop(&mut self, binding_id: &str, hotkey_string: &str) {
        let Some(action) = ACTION_MAP.get(binding_id) else {
            warn!("No action in ACTION_MAP for '{binding_id}'");
            return;
        };
        let operation = self.next_operation();
        self.stage = Stage::Processing {
            operation: operation.clone(),
        };
        action.stop(&self.app, binding_id, hotkey_string, operation);
    }
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
        let worker = Worker {
            app,
            shutdown_drain: Arc::clone(&shutdown_drain),
            stage: Stage::Idle,
            gesture: Gesture::Idle,
            pending_start: None,
            next_operation_id: 1,
            last_press: None,
        };

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker.run(rx)));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            } else {
                debug!("Transcription coordinator exited");
            }
        });

        Self { tx, shutdown_drain }
    }

    fn send(&self, command: Command) {
        if self.shutdown_drain.is_closing() {
            debug!("Ignoring transcription input while the application is closing");
            return;
        }
        if self.tx.send(command).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// Forwards a press or release of the recording shortcut.
    pub fn send_key(&self, binding_id: &str, hotkey_string: &str, is_pressed: bool) {
        self.send(Command::Key {
            binding_id: binding_id.to_string(),
            hotkey_string: hotkey_string.to_string(),
            key: if is_pressed { Key::Press } else { Key::Release },
            at: Instant::now(),
        });
    }

    /// Starts a hands-free recording, or stops the current one, for the CLI
    /// and SIGUSR2.
    pub fn send_toggle(&self, binding_id: &str, source: &str) {
        self.send(Command::Toggle {
            binding_id: binding_id.to_string(),
            hotkey_string: source.to_string(),
        });
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

    pub fn send_paste_last(&self) {
        self.send(Command::PasteLast);
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
            // A refresh request times out on its own within this.
            if !crate::codex_auth::wait_for_refreshes(crate::codex_auth::REFRESH_TIMEOUT) {
                warn!("Timed out waiting for a Codex token refresh during shutdown");
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
            limit_warned: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn recording_stage(deadline: Instant) -> Stage {
        Stage::Recording(ActiveRecording {
            binding_id: "transcribe".to_string(),
            hotkey_string: "CLI".to_string(),
            deadline,
            limit_warned: false,
        })
    }

    fn pending_start(deadline: Instant) -> PendingStart {
        PendingStart {
            binding_id: "transcribe".to_string(),
            hotkey_string: "Option+Space".to_string(),
            deadline,
        }
    }

    #[test]
    fn remote_cancel_after_toggle_returns_the_coordinator_to_idle() {
        let mut stage = recording_stage(Instant::now() + MAX_RECORDING_DURATION);
        let mut pending = Some(pending_start(Instant::now()));

        finish_cancel(&mut stage, &mut pending);

        assert!(matches!(stage, Stage::Idle));
        assert!(pending.is_none());
    }

    #[test]
    fn processing_cancel_returns_to_idle_while_audio_is_still_stopping() {
        let mut stage = Stage::Processing {
            operation: ProcessingOperation::new(OperationId(1)),
        };

        finish_cancel(&mut stage, &mut None);

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
    fn receiver_wakes_for_the_earliest_gesture_start_or_recording_deadline() {
        let now = Instant::now();
        let stage = recording_stage(now + Duration::from_secs(600));
        let waiting = Gesture::AwaitingSecondTap {
            deadline: now + Duration::from_millis(350),
        };
        assert_eq!(
            next_deadline(&stage, waiting, None),
            Some(now + Duration::from_millis(350))
        );
        let retry = pending_start(now + Duration::from_millis(25));
        assert_eq!(
            next_deadline(&stage, waiting, Some(&retry)),
            Some(now + Duration::from_millis(25))
        );
        assert_eq!(next_deadline(&Stage::Idle, Gesture::Idle, None), None);
    }

    #[test]
    fn recording_warns_a_minute_before_the_cap_then_stops_at_it() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(600);
        let mut stage = recording_stage(deadline);
        let warn_at = deadline - LIMIT_WARNING_LEAD;

        assert_eq!(
            next_deadline(&stage, Gesture::HandsFree { key_down: false }, None),
            Some(warn_at)
        );
        assert!(!take_due_limit_warning(
            &mut stage,
            warn_at - Duration::from_millis(1)
        ));
        assert!(take_due_limit_warning(&mut stage, warn_at));
        assert!(
            !take_due_limit_warning(&mut stage, warn_at),
            "the warning is shown once"
        );
        assert_eq!(
            next_deadline(&stage, Gesture::HandsFree { key_down: false }, None),
            Some(deadline)
        );
        assert_eq!(
            take_due_limit_stop(&stage, deadline - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            take_due_limit_stop(&stage, deadline),
            Some(("transcribe".to_string(), "CLI".to_string()))
        );
    }

    #[test]
    fn deferred_start_waits_for_idle_and_never_creates_a_phantom_recording() {
        let now = Instant::now();
        let mut pending = Some(pending_start(now));

        // Cancellation can leave the coordinator Idle while audio is Stopping.
        // Retrying remains a request until the recorder admits it.
        assert_eq!(
            take_due_start(&Stage::Idle, &mut pending, now),
            Some(("transcribe".to_string(), "Option+Space".to_string()))
        );
        assert!(pending.is_none());

        let mut stale = Some(pending_start(now));
        assert_eq!(take_due_start(&recording_stage(now), &mut stale, now), None);
        assert!(stale.is_none());
    }

    #[test]
    fn a_gesture_outlives_neither_its_recording_nor_its_queued_start() {
        let now = Instant::now();
        assert!(gesture_has_recording(&recording_stage(now), None));
        assert!(gesture_has_recording(
            &Stage::Idle,
            Some(&pending_start(now))
        ));
        assert!(!gesture_has_recording(&Stage::Idle, None));
        assert!(!gesture_has_recording(
            &Stage::Processing {
                operation: ProcessingOperation::new(OperationId(1)),
            },
            None
        ));
    }

    /// Feeds the queue through `next_step` the way `Worker::run` does, at a
    /// fixed `now`, and returns the intents and the final gesture.
    fn drain_at(
        rx: &Receiver<Command>,
        mut gesture: Gesture,
        now: Instant,
    ) -> (Vec<Intent>, Gesture) {
        let mut intents = Vec::new();
        loop {
            match next_step(rx, gesture, now) {
                Next::Command(Command::Key { key, at, .. }) => {
                    let (next, intent) = gesture.on_key(key, at);
                    gesture = next;
                    intents.push(intent);
                }
                Next::GestureExpired(next) => {
                    gesture = next;
                    intents.push(Intent::Discard);
                }
                Next::Command(_) => unreachable!("only key commands are queued"),
                Next::Wait | Next::Closed => break,
            }
        }
        intents.retain(|intent| *intent != Intent::None);
        (intents, gesture)
    }

    fn key_at(key: Key, at: Instant) -> Command {
        Command::Key {
            binding_id: "transcribe".to_string(),
            hotkey_string: "Option+Space".to_string(),
            key,
            at,
        }
    }

    #[test]
    fn a_double_tap_queued_behind_a_slow_microphone_locks_hands_free() {
        let t0 = Instant::now();
        let (tx, rx) = mpsc::channel();
        // The first press started a recording, and opening the microphone
        // held the coordinator until 600 ms while the rest of the double tap
        // queued up.
        for (key, at_ms) in [(Key::Release, 80), (Key::Press, 200), (Key::Release, 260)] {
            tx.send(key_at(key, t0 + Duration::from_millis(at_ms)))
                .unwrap();
        }

        let (intents, gesture) = drain_at(
            &rx,
            Gesture::Pressed { since: t0 },
            t0 + Duration::from_millis(600),
        );

        assert_eq!(intents, [Intent::LockHandsFree]);
        assert_eq!(gesture, Gesture::HandsFree { key_down: false });
    }

    #[test]
    fn a_lone_tap_expires_only_once_the_queue_is_empty() {
        let t0 = Instant::now();
        let (tx, rx) = mpsc::channel();
        tx.send(key_at(Key::Release, t0 + Duration::from_millis(80)))
            .unwrap();

        let (intents, gesture) = drain_at(
            &rx,
            Gesture::Pressed { since: t0 },
            t0 + Duration::from_millis(600),
        );

        assert_eq!(intents, [Intent::Discard]);
        assert_eq!(gesture, Gesture::Idle);
    }

    #[test]
    fn presses_closer_than_the_debounce_are_bounces() {
        let t0 = Instant::now();
        assert!(!is_bounce(None, t0));
        assert!(is_bounce(Some(t0), t0 + Duration::from_millis(29)));
        assert!(!is_bounce(Some(t0), t0 + DEBOUNCE));
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
}
