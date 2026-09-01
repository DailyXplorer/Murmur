use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use log::{debug, error, warn};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);
const RELEASE_GRACE: Duration = Duration::from_millis(50);
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
    ProcessingFinished,
}

/// Pipeline lifecycle, owned exclusively by the coordinator thread.
enum Stage {
    Idle,
    Recording(ActiveRecording),
    Processing,
}

fn next_deadline(stage: &Stage, pending_release: Option<&PendingRelease>) -> Option<Instant> {
    let recording_deadline = match stage {
        Stage::Recording(recording) => Some(recording.deadline),
        _ => None,
    };
    match (
        pending_release.map(|pending| pending.deadline),
        recording_deadline,
    ) {
        (Some(release), Some(recording)) => Some(release.min(recording)),
        (Some(release), None) => Some(release),
        (None, Some(recording)) => Some(recording),
        (None, None) => None,
    }
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

fn cancellation_stage(stage: &Stage) -> crate::utils::CancellationStage {
    if matches!(stage, Stage::Processing) {
        crate::utils::CancellationStage::Processing
    } else {
        crate::utils::CancellationStage::NotProcessing
    }
}

fn finish_cancel(stage: &mut Stage, pending_release: &mut Option<PendingRelease>) {
    *pending_release = None;
    if !matches!(stage, Stage::Processing) {
        *stage = Stage::Idle;
    }
}

fn queue_local_cancel(tx: &Sender<Command>) -> Result<(), mpsc::SendError<Command>> {
    tx.send(Command::Cancel)
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
}

/// Returns whether `id` names the sole supported transcription shortcut.
pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe"
}

impl TranscriptionCoordinator {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut stage = Stage::Idle;
                let mut last_press: Option<Instant> = None;
                let mut pending_release: Option<PendingRelease> = None;

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
                        stop(&app, &mut stage, &binding_id, &hotkey_string);
                        continue;
                    }

                    let cmd = if let Some(deadline) =
                        next_deadline(&stage, pending_release.as_ref())
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
                                    start(&app, &mut stage, &binding_id, &hotkey_string);
                                } else if !is_pressed
                                    && matches!(&stage, Stage::Recording(recording) if recording.binding_id == binding_id)
                                {
                                    stop(&app, &mut stage, &binding_id, &hotkey_string);
                                }
                            } else if is_pressed {
                                match &stage {
                                    Stage::Idle => {
                                        start(&app, &mut stage, &binding_id, &hotkey_string);
                                    }
                                    Stage::Recording(recording)
                                        if recording.binding_id == binding_id =>
                                    {
                                        stop(&app, &mut stage, &binding_id, &hotkey_string);
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
                            let cancellation_stage = cancellation_stage(&stage);
                            crate::utils::cancel_current_operation_from_coordinator(
                                &app,
                                cancellation_stage,
                            );
                            finish_cancel(&mut stage, &mut pending_release);
                        }
                        Command::ProcessingFinished => {
                            stage = Stage::Idle;
                        }
                    }
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self { tx }
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

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }
}

fn start(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.start(app, binding_id, hotkey_string);
    if app
        .try_state::<Arc<AudioRecordingManager>>()
        .is_some_and(|a| a.is_recording())
    {
        *stage = Stage::Recording(ActiveRecording {
            binding_id: binding_id.to_string(),
            hotkey_string: hotkey_string.to_string(),
            deadline: Instant::now() + MAX_RECORDING_DURATION,
        });
    } else {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
}

fn stop(app: &AppHandle, stage: &mut Stage, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.stop(app, binding_id, hotkey_string);
    *stage = Stage::Processing;
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

        finish_cancel(&mut stage, &mut pending_release);

        assert!(matches!(stage, Stage::Idle));
        assert!(pending_release.is_none());
    }

    #[test]
    fn remote_cancel_keeps_processing_stage_while_audio_is_still_stopping() {
        let mut stage = Stage::Processing;
        let mut pending_release = None;

        // `stop` moves the coordinator to Processing before its async worker
        // stops the audio manager, so that manager can still report active.
        finish_cancel(&mut stage, &mut pending_release);

        assert!(matches!(stage, Stage::Processing));
    }

    #[test]
    fn local_cancel_during_processing_uses_stage_not_active_audio_for_ui_cleanup() {
        let mut stage = Stage::Processing;
        let mut pending_release = None;

        // `stop` enters Processing before its worker releases the audio
        // manager. The cancellation must still be signalled, but none of the
        // idle UI side effects may run before FinishGuard completes.
        let effects = crate::utils::cancellation_effects(cancellation_stage(&stage), true);
        assert!(effects.signal_audio_cancellation);
        assert!(!effects.unregister_cancel_shortcut);
        assert!(!effects.set_tray_idle);
        assert!(!effects.hide_recording_overlay);

        finish_cancel(&mut stage, &mut pending_release);
        assert!(matches!(stage, Stage::Processing));
    }

    #[test]
    fn local_cancel_during_recording_cleans_up_immediately() {
        let mut stage = recording_stage(Instant::now() + MAX_RECORDING_DURATION);
        let mut pending_release = None;

        let effects = crate::utils::cancellation_effects(cancellation_stage(&stage), true);
        assert!(effects.signal_audio_cancellation);
        assert!(effects.unregister_cancel_shortcut);
        assert!(effects.set_tray_idle);
        assert!(effects.hide_recording_overlay);

        finish_cancel(&mut stage, &mut pending_release);
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
            next_deadline(&stage, Some(&pending)),
            Some(pending.deadline)
        );
        assert_eq!(
            next_deadline(&stage, None),
            Some(now + Duration::from_secs(10))
        );
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
