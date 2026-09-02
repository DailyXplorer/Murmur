use crate::input::EnigoState;
use crate::overlay;
use crate::shortcut;
use crate::tray::{change_tray_icon, TrayIconState};
use crate::utils;
use crate::TranscriptionCoordinator;
use anyhow::{anyhow, Context, Result};
use enigo::{Direction, Key, Keyboard};
use log::warn;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

const META_AI_BUNDLE_ID: &str = "com.meta.endo.vanilla";
const META_AI_DOWNLOAD_URL: &str = "https://www.meta.ai/download/";
const ARMING_TIMEOUT: Duration = Duration::from_secs(5);
const FINALIZATION_TIMEOUT: Duration = Duration::from_secs(15);
const PILL_POLL_INTERVAL: Duration = Duration::from_millis(16);
const PILL_MISSING_POLLS: u8 = 10;
const PILL_MOVE_FAILURE_POLLS: u8 = 3;
const STARTING_SETTLE_TIME: Duration = Duration::from_secs(2);
const SHUTDOWN_RELEASE_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizationDecision {
    Finish,
    Wait,
    ReportStuckPill,
}

/// A seen Meta indicator keeps ownership of its generation until it actually
/// disappears. A timeout is diagnostic only: releasing the coordinator while
/// the pill remains visible could overlap another Fn-down with live Meta
/// dictation.
fn finalization_decision(
    pill_seen: bool,
    missing_polls: u8,
    elapsed: Duration,
    timeout_reported: bool,
) -> FinalizationDecision {
    if pill_seen {
        if missing_polls >= PILL_MISSING_POLLS {
            FinalizationDecision::Finish
        } else if elapsed >= FINALIZATION_TIMEOUT && !timeout_reported {
            FinalizationDecision::ReportStuckPill
        } else {
            FinalizationDecision::Wait
        }
    } else if elapsed >= STARTING_SETTLE_TIME {
        // Fn-up succeeded but no Meta pill was ever observed, so there is no
        // active visual generation left to serialize against.
        FinalizationDecision::Finish
    } else {
        FinalizationDecision::Wait
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MetaAppRuntimeState {
    NotRunning,
    Active,
    WindowVisible,
    Dictating,
    InspectionUnavailable,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MetaAppErrorCode {
    SetupRequired,
    TargetChanged,
    OverlayUnavailable,
    KeyboardControlFailed,
    InspectionUnavailable,
    IndicatorMoveFailed,
    DictationDidNotStart,
    DictationEndedUnexpectedly,
    IndicatorStuck,
    ExitReleaseFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct MetaAppErrorEvent {
    pub code: MetaAppErrorCode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MetaUiSnapshot {
    pill_present: bool,
    non_pill_window_visible: bool,
}

fn classify_runtime_state(
    running: bool,
    active: bool,
    snapshot: Option<MetaUiSnapshot>,
) -> MetaAppRuntimeState {
    if !running {
        return MetaAppRuntimeState::NotRunning;
    }
    if active {
        return MetaAppRuntimeState::Active;
    }
    let Some(snapshot) = snapshot else {
        return MetaAppRuntimeState::InspectionUnavailable;
    };
    if snapshot.non_pill_window_visible {
        MetaAppRuntimeState::WindowVisible
    } else if snapshot.pill_present {
        MetaAppRuntimeState::Dictating
    } else {
        MetaAppRuntimeState::Ready
    }
}

fn non_pill_window_is_visible(minimized: Option<bool>) -> bool {
    minimized != Some(true)
}

fn target_still_focused(target: TargetApplication, frontmost_pid: Option<i32>) -> bool {
    frontmost_pid == Some(target.pid)
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MetaAppStatus {
    pub installed: bool,
    pub dictation_enabled: bool,
    pub hold_fn_enabled: bool,
    pub accessibility_trusted: bool,
    pub runtime_state: MetaAppRuntimeState,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetApplication {
    pid: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DictationRun {
    generation: u64,
    target: TargetApplication,
}

/// The one authoritative lifecycle for a Meta AI app dictation session.
///
/// `Recording` and `ReleaseBlocked` are the only states in which Fn may still
/// be held. Keeping that fact in the type prevents cleanup paths from becoming
/// idle before a successful Fn-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    Idle,
    Starting {
        generation: u64,
    },
    Recording {
        generation: u64,
    },
    ReleaseBlocked {
        generation: u64,
    },
    Finalizing {
        generation: u64,
        since: Instant,
        pill_ownership: PillOwnership,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PillOwnership {
    None,
    Possible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaStopDisposition {
    /// Fn-up succeeded (or Fn was never pressed) and the bridge owns the
    /// asynchronous finalization.
    Finalizing,
    /// Another caller already requested a successful stop.
    AlreadyFinalizing,
    /// The bridge has no Meta session; callers may use the audio path instead.
    Inactive,
}

impl Session {
    fn generation(self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Starting { generation }
            | Self::Recording { generation }
            | Self::ReleaseBlocked { generation }
            | Self::Finalizing { generation, .. } => Some(generation),
        }
    }

    fn begin(&mut self, generation: u64) -> bool {
        if !matches!(*self, Self::Idle) {
            return false;
        }
        *self = Self::Starting { generation };
        true
    }

    fn is_starting_generation(self, generation: u64) -> bool {
        matches!(self, Self::Starting { generation: current } if current == generation)
    }

    /// Shutdown has no Fn-up obligation for `Starting`, but it must revoke the
    /// detached worker's authority to press Fn before the app can exit.
    fn invalidate_starting_for_shutdown(&mut self) -> bool {
        if matches!(*self, Self::Starting { .. }) {
            *self = Self::Idle;
            true
        } else {
            false
        }
    }

    /// Attempts Fn-down and, on an error, immediately sends the compensating
    /// Fn-up. A failed key-down may already have reached macOS, so it is never
    /// safe to return to `Idle` without that compensation succeeding.
    fn activate<P, R>(&mut self, generation: u64, press_fn: P, release_fn: R) -> Result<bool>
    where
        P: FnOnce() -> Result<()>,
        R: FnOnce() -> Result<()>,
    {
        if !matches!(*self, Self::Starting { generation: current } if current == generation) {
            return Ok(false);
        }

        match press_fn() {
            Ok(()) => {
                *self = Self::Recording { generation };
                Ok(true)
            }
            Err(press_error) => {
                *self = Self::ReleaseBlocked { generation };
                match release_fn() {
                    Ok(()) => {
                        *self = Self::Finalizing {
                            generation,
                            since: Instant::now(),
                            pill_ownership: PillOwnership::Possible,
                        };
                        Err(press_error)
                    }
                    Err(release_error) => Err(anyhow!(
                        "Failed to press Meta AI dictation Fn ({press_error}); compensating Fn release also failed ({release_error})"
                    )),
                }
            }
        }
    }

    /// Stops the session while preserving retryability. `ReleaseBlocked` is
    /// deliberately retried on the next stop, cancel, or shutdown attempt.
    fn request_stop<F>(&mut self, release_fn: F) -> Result<bool>
    where
        F: FnOnce() -> Result<()>,
    {
        match *self {
            Self::Idle => Ok(false),
            Self::Starting { generation } => {
                *self = Self::Finalizing {
                    generation,
                    since: Instant::now() - STARTING_SETTLE_TIME,
                    pill_ownership: PillOwnership::None,
                };
                Ok(true)
            }
            Self::Recording { generation } | Self::ReleaseBlocked { generation } => {
                match release_fn() {
                    Ok(()) => {
                        *self = Self::Finalizing {
                            generation,
                            since: Instant::now(),
                            pill_ownership: PillOwnership::Possible,
                        };
                        Ok(true)
                    }
                    Err(error) => {
                        *self = Self::ReleaseBlocked { generation };
                        Err(error)
                    }
                }
            }
            Self::Finalizing { .. } => Ok(false),
        }
    }

    /// Requests failure cleanup for one monitor generation. A monitor may
    /// complete only a matching `Finalizing` session, never a recording one.
    fn fail<F>(&mut self, generation: u64, release_fn: F) -> Result<bool>
    where
        F: FnOnce() -> Result<()>,
    {
        match *self {
            Self::Starting {
                generation: current,
            } if current == generation => {
                *self = Self::Finalizing {
                    generation,
                    since: Instant::now() - STARTING_SETTLE_TIME,
                    pill_ownership: PillOwnership::None,
                };
                Ok(true)
            }
            Self::Recording {
                generation: current,
            }
            | Self::ReleaseBlocked {
                generation: current,
            } if current == generation => self.request_stop(release_fn),
            _ => Ok(false),
        }
    }

    /// Returns true only when the matching generation has already released Fn
    /// and may safely give the next shortcut slot back to the coordinator.
    fn finish(&mut self, generation: u64) -> bool {
        if matches!(*self, Self::Finalizing { generation: current, .. } if current == generation) {
            *self = Self::Idle;
            true
        } else {
            false
        }
    }

    fn fn_may_be_held(self) -> bool {
        matches!(self, Self::Recording { .. } | Self::ReleaseBlocked { .. })
    }

    fn release_for_shutdown<F>(&mut self, attempts: usize, mut release_fn: F) -> Result<bool>
    where
        F: FnMut() -> Result<()>,
    {
        if !self.fn_may_be_held() {
            return Ok(false);
        }

        let mut last_error = None;
        for _ in 0..attempts {
            match self.request_stop(&mut release_fn) {
                Ok(true) => return Ok(true),
                Ok(false) => return Ok(false),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("No shutdown release attempts were made")))
    }

    fn finalizing_since(self) -> Option<Instant> {
        match self {
            Self::Finalizing { since, .. } => Some(since),
            _ => None,
        }
    }

    fn pill_may_be_owned(self) -> bool {
        match self {
            Self::Recording { .. } | Self::ReleaseBlocked { .. } => true,
            Self::Finalizing { pill_ownership, .. } => pill_ownership == PillOwnership::Possible,
            Self::Idle | Self::Starting { .. } => false,
        }
    }
}

/// Coordinates Murmur's shortcut and overlay with Meta AI's global Fn
/// dictation. Meta AI owns microphone capture and text insertion; Murmur never
/// reads Meta's credentials or private network protocol.
pub struct MetaAppBridge {
    next_generation: AtomicU64,
    session: Mutex<Session>,
}

impl Default for MetaAppBridge {
    fn default() -> Self {
        Self {
            next_generation: AtomicU64::new(0),
            session: Mutex::new(Session::Idle),
        }
    }
}

fn report_error(app: &AppHandle, code: MetaAppErrorCode, detail: impl std::fmt::Display) {
    warn!("Meta AI app dictation error ({code:?}): {detail}");
    let _ = (MetaAppErrorEvent { code }).emit(app);
}

fn reported_failure<T>(
    app: &AppHandle,
    code: MetaAppErrorCode,
    detail: impl Into<String>,
) -> Result<T> {
    let detail = detail.into();
    report_error(app, code, &detail);
    Err(anyhow!(detail))
}

pub(crate) fn report_exit_release_failure(app: &AppHandle, detail: impl std::fmt::Display) {
    report_error(app, MetaAppErrorCode::ExitReleaseFailed, detail);
}

impl MetaAppBridge {
    pub fn start(&self, app: &AppHandle) -> Result<bool> {
        let status = status();
        if !status.installed {
            return reported_failure(
                app,
                MetaAppErrorCode::SetupRequired,
                format!(
                    "Meta AI for Mac is not installed. Install it from {META_AI_DOWNLOAD_URL}."
                ),
            );
        }
        if !status.dictation_enabled {
            return reported_failure(
                app,
                MetaAppErrorCode::SetupRequired,
                "Enable Dictation in Meta AI settings before using it from Murmur.",
            );
        }
        if !status.hold_fn_enabled {
            return reported_failure(
                app,
                MetaAppErrorCode::SetupRequired,
                "Set Meta AI's hold-to-dictate shortcut to Fn before using it from Murmur.",
            );
        }
        if !status.accessibility_trusted {
            return reported_failure(
                app,
                MetaAppErrorCode::SetupRequired,
                "Murmur needs Accessibility permission before using Meta AI dictation.",
            );
        }
        match status.runtime_state {
            MetaAppRuntimeState::Ready => {}
            MetaAppRuntimeState::NotRunning => {
                return reported_failure(
                    app,
                    MetaAppErrorCode::SetupRequired,
                    "Open Meta AI, close its main window so it stays in the menu bar, then retry.",
                );
            }
            MetaAppRuntimeState::Active | MetaAppRuntimeState::WindowVisible => {
                return reported_failure(
                    app,
                    MetaAppErrorCode::SetupRequired,
                    "Close the Meta AI window and return to the app where you want to type before starting dictation.",
                );
            }
            MetaAppRuntimeState::Dictating => {
                return reported_failure(
                    app,
                    MetaAppErrorCode::SetupRequired,
                    "Meta AI is already dictating. Finish that dictation before starting Murmur.",
                );
            }
            MetaAppRuntimeState::InspectionUnavailable => {
                return reported_failure(
                    app,
                    MetaAppErrorCode::InspectionUnavailable,
                    "Murmur could not verify that Meta AI is safely in the background.",
                );
            }
        }

        let target = match macos::frontmost_target() {
            Ok(target) => target,
            Err(error) => {
                return reported_failure(app, MetaAppErrorCode::TargetChanged, error.to_string())
            }
        };

        let generation = self
            .next_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        {
            let Ok(mut session) = self.session.lock() else {
                return reported_failure(
                    app,
                    MetaAppErrorCode::InspectionUnavailable,
                    "Meta AI bridge state is unavailable",
                );
            };
            if !session.begin(generation) {
                return Ok(false);
            }
        }

        let run = DictationRun { generation, target };
        show_recording_ui(app);
        self.spawn_pill_monitor(app.clone(), run);
        self.spawn_start_worker(app.clone(), run);
        Ok(true)
    }

    pub(crate) fn stop(&self, app: &AppHandle) -> Result<MetaStopDisposition> {
        let result = {
            let Ok(mut session) = self.session.lock() else {
                return reported_failure(
                    app,
                    MetaAppErrorCode::InspectionUnavailable,
                    "Meta AI bridge state is unavailable",
                );
            };
            session.request_stop(|| set_function_key(app, Direction::Release))
        };

        match result {
            Ok(true) => {
                show_finalizing_ui(app);
                Ok(MetaStopDisposition::Finalizing)
            }
            Ok(false) => {
                let Ok(session) = self.session.lock() else {
                    return reported_failure(
                        app,
                        MetaAppErrorCode::InspectionUnavailable,
                        "Meta AI bridge state is unavailable",
                    );
                };
                Ok(if matches!(*session, Session::Finalizing { .. }) {
                    MetaStopDisposition::AlreadyFinalizing
                } else {
                    MetaStopDisposition::Inactive
                })
            }
            Err(error) => {
                report_error(
                    app,
                    MetaAppErrorCode::KeyboardControlFailed,
                    format!("Failed to release Meta AI dictation: {error}"),
                );
                Err(error)
            }
        }
    }

    fn emit_recording_ready_if_current(&self, app: &AppHandle, generation: u64) -> bool {
        let Ok(session) = self.session.lock() else {
            return false;
        };
        if !matches!(*session, Session::Recording { generation: current } if current == generation)
        {
            return false;
        }

        // Keep the generation lock until the UI event is queued. A stop or
        // later generation cannot interleave between this check and the emit.
        overlay::emit_meta_app_recording_ready(app);
        true
    }

    fn is_starting_generation(&self, generation: u64) -> bool {
        self.session
            .lock()
            .map(|session| session.is_starting_generation(generation))
            .unwrap_or(false)
    }

    pub fn prepare_exit(&self, app: &AppHandle) -> Result<()> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("Meta AI bridge state is unavailable during shutdown"))?;

        if session.invalidate_starting_for_shutdown() {
            return Ok(());
        }

        session
            .release_for_shutdown(SHUTDOWN_RELEASE_ATTEMPTS, || {
                set_function_key(app, Direction::Release)
            })
            .map(|_| ())
    }

    fn spawn_start_worker(&self, app: AppHandle, run: DictationRun) {
        std::thread::spawn(move || {
            let generation = run.generation;
            let still_starting = || {
                app.state::<MetaAppBridge>()
                    .is_starting_generation(generation)
            };
            if !still_starting() {
                return;
            }

            // Never press Fn until Murmur's non-activating panel is actually
            // on screen. This prevents Meta's indicator from flashing before
            // the window that will cover it exists.
            let overlay_deadline = Instant::now() + Duration::from_secs(1);
            let mut overlay_frame = None;
            while Instant::now() < overlay_deadline {
                if !still_starting() {
                    return;
                }
                overlay_frame = overlay::meta_overlay_frame(&app);
                if overlay_frame.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            if overlay_frame.is_none() {
                fail(
                    &app,
                    generation,
                    MetaAppErrorCode::OverlayUnavailable,
                    "Murmur could not show its recording overlay.".to_string(),
                );
                return;
            }
            if !still_starting() {
                return;
            }
            if let Err(error) = macos::ensure_ready_for_target(run.target, false) {
                fail(
                    &app,
                    generation,
                    MetaAppErrorCode::SetupRequired,
                    error.to_string(),
                );
                return;
            }
            if !still_starting() {
                return;
            }

            let press_result = {
                let bridge = app.state::<MetaAppBridge>();
                let Ok(mut session) = bridge.session.lock() else {
                    return;
                };
                session.activate(
                    generation,
                    || set_function_key(&app, Direction::Press),
                    || set_function_key(&app, Direction::Release),
                )
            };

            let pressed = match press_result {
                Ok(pressed) => pressed,
                Err(error) => {
                    let finalizing = app
                        .state::<MetaAppBridge>()
                        .session
                        .lock()
                        .is_ok_and(|session| matches!(*session, Session::Finalizing { generation: current, .. } if current == generation));
                    if finalizing {
                        show_finalizing_ui(&app);
                    }
                    report_error(&app, MetaAppErrorCode::KeyboardControlFailed, error);
                    return;
                }
            };

            if pressed {
                if let Err(error) = macos::ensure_ready_for_target(run.target, true) {
                    fail(
                        &app,
                        generation,
                        MetaAppErrorCode::TargetChanged,
                        format!(
                            "The focused app or Meta AI changed while dictation started. Murmur stopped dictation: {error}"
                        ),
                    );
                }
            }
        });
    }

    fn spawn_pill_monitor(&self, app: AppHandle, run: DictationRun) {
        std::thread::spawn(move || {
            let generation = run.generation;
            let mut pill_seen = false;
            let mut missing_polls = 0_u8;
            let mut move_failure_polls = 0_u8;
            let mut recording_started = None;
            let mut ready_emitted = false;
            let mut finalization_timeout_reported = false;
            let mut invariant_failure_reported = false;

            loop {
                let session = {
                    let bridge = app.state::<MetaAppBridge>();
                    let Ok(session) = bridge.session.lock() else {
                        break;
                    };
                    if session.generation() != Some(generation) {
                        break;
                    }
                    *session
                };

                // A release failure is deliberately sticky. The monitor must
                // not repeatedly emit the same failure while a user-triggered
                // stop/cancel/shutdown retry still owns Fn-up.
                if matches!(session, Session::ReleaseBlocked { .. }) {
                    std::thread::sleep(PILL_POLL_INTERVAL);
                    continue;
                }

                if matches!(session, Session::Recording { .. }) && recording_started.is_none() {
                    recording_started = Some(Instant::now());
                }

                let pill_may_be_owned = session.pill_may_be_owned();
                if pill_may_be_owned {
                    if let Err(error) = macos::ensure_ready_for_target(run.target, true) {
                        let message = format!(
                            "The focused app changed or Meta AI exposed a window during dictation: {error}"
                        );
                        if matches!(session, Session::Recording { .. }) {
                            invariant_failure_reported = true;
                            fail(&app, generation, MetaAppErrorCode::TargetChanged, message);
                            continue;
                        }
                        if !invariant_failure_reported {
                            invariant_failure_reported = true;
                            report_error(&app, MetaAppErrorCode::TargetChanged, message);
                        }
                        std::thread::sleep(PILL_POLL_INTERVAL);
                        continue;
                    }
                }

                if pill_may_be_owned {
                    let observation = match overlay::meta_overlay_frame(&app) {
                        Some(frame) => macos::position_dictation_pill(frame),
                        None => Err(anyhow!("Murmur's Meta AI overlay is unavailable")),
                    };
                    let observation = match observation {
                        Ok(observation) => observation,
                        Err(error) => {
                            if matches!(session, Session::Recording { .. }) {
                                invariant_failure_reported = true;
                                fail(
                                    &app,
                                    generation,
                                    MetaAppErrorCode::InspectionUnavailable,
                                    error.to_string(),
                                );
                            } else if !invariant_failure_reported {
                                invariant_failure_reported = true;
                                report_error(&app, MetaAppErrorCode::InspectionUnavailable, error);
                            }
                            std::thread::sleep(PILL_POLL_INTERVAL);
                            continue;
                        }
                    };

                    pill_seen |= observation.present;
                    if observation.present && observation.moved {
                        move_failure_polls = 0;
                        missing_polls = 0;
                        if !ready_emitted && matches!(session, Session::Recording { .. }) {
                            let bridge = app.state::<MetaAppBridge>();
                            ready_emitted =
                                bridge.emit_recording_ready_if_current(&app, generation);
                        }
                    } else if observation.present {
                        move_failure_polls = move_failure_polls.saturating_add(1);
                    } else if pill_seen {
                        missing_polls = missing_polls.saturating_add(1);
                    }
                }

                if matches!(session, Session::Recording { .. })
                    && move_failure_polls >= PILL_MOVE_FAILURE_POLLS
                {
                    fail(
                        &app,
                        generation,
                        MetaAppErrorCode::IndicatorMoveFailed,
                        "Murmur could not hide Meta AI's dictation indicator. Dictation was stopped."
                            .to_string(),
                    );
                    continue;
                }

                if matches!(session, Session::Recording { .. }) {
                    let arming_elapsed = recording_started
                        .map(|started| started.elapsed())
                        .unwrap_or_default();
                    if !pill_seen && arming_elapsed >= ARMING_TIMEOUT {
                        fail(
                            &app,
                            generation,
                            MetaAppErrorCode::DictationDidNotStart,
                            "Meta AI did not start dictation. Open Meta AI, check its session and Dictation settings, then retry."
                                .to_string(),
                        );
                        continue;
                    }
                    if pill_seen && missing_polls >= PILL_MISSING_POLLS {
                        fail(
                            &app,
                            generation,
                            MetaAppErrorCode::DictationEndedUnexpectedly,
                            "Meta AI dictation ended unexpectedly.".to_string(),
                        );
                        continue;
                    }
                }

                if let Some(finalizing_since) = session.finalizing_since() {
                    match finalization_decision(
                        pill_seen,
                        missing_polls,
                        finalizing_since.elapsed(),
                        finalization_timeout_reported,
                    ) {
                        FinalizationDecision::Finish => {
                            finish(&app, generation);
                            break;
                        }
                        FinalizationDecision::ReportStuckPill => {
                            finalization_timeout_reported = true;
                            let message = "Meta AI's dictation indicator is still visible after Fn was released. Murmur will stay in its finishing state; close or finish Meta AI dictation before starting another one.";
                            report_error(&app, MetaAppErrorCode::IndicatorStuck, message);
                        }
                        FinalizationDecision::Wait => {}
                    }
                }

                std::thread::sleep(PILL_POLL_INTERVAL);
            }
        });
    }
}

fn fail(app: &AppHandle, generation: u64, code: MetaAppErrorCode, message: String) {
    let result = {
        let bridge = app.state::<MetaAppBridge>();
        let Ok(mut session) = bridge.session.lock() else {
            return;
        };
        session.fail(generation, || set_function_key(app, Direction::Release))
    };

    match result {
        Ok(true) => {
            show_finalizing_ui(app);
            report_error(app, code, message);
        }
        Ok(false) => {}
        Err(error) => {
            report_error(
                app,
                MetaAppErrorCode::KeyboardControlFailed,
                format!("{message} Failed to release Meta AI dictation: {error}"),
            );
        }
    }
}

fn finish(app: &AppHandle, generation: u64) {
    let should_finish = {
        let bridge = app.state::<MetaAppBridge>();
        let Ok(mut session) = bridge.session.lock() else {
            return;
        };
        session.finish(generation)
    };

    if !should_finish {
        return;
    }

    finish_ui(app);
}

fn show_recording_ui(app: &AppHandle) {
    change_tray_icon(app, TrayIconState::Recording);
    overlay::show_meta_app_recording_overlay(app);
    shortcut::register_cancel_shortcut(app);
}

fn show_finalizing_ui(app: &AppHandle) {
    change_tray_icon(app, TrayIconState::Transcribing);
    overlay::show_meta_app_transcribing_overlay(app);
}

fn finish_ui(app: &AppHandle) {
    shortcut::unregister_cancel_shortcut(app);
    utils::hide_recording_overlay(app);
    change_tray_icon(app, TrayIconState::Idle);
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.notify_processing_finished();
    }
}

fn set_function_key(app: &AppHandle, direction: Direction) -> Result<()> {
    let state = app
        .try_state::<EnigoState>()
        .ok_or_else(|| anyhow!("Murmur needs Accessibility permission before using Meta AI"))?;
    let mut enigo = state
        .0
        .lock()
        .map_err(|_| anyhow!("Murmur's keyboard controller is unavailable"))?;
    enigo
        .key(Key::Function, direction)
        .map_err(|error| anyhow!("Failed to control Meta AI dictation: {error}"))
}

pub fn status() -> MetaAppStatus {
    let installed = macos::is_installed();
    let accessibility_trusted = macos::accessibility_trusted();
    let runtime_state = macos::runtime_state();
    let dictation_enabled = installed
        && read_meta_default("endo_vanilla_dictation_enabled")
            .is_some_and(|value| parse_default_bool(&value));
    let hold_fn_enabled = installed
        && read_meta_default("endo_dictation_key")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("fn"));

    MetaAppStatus {
        installed,
        dictation_enabled,
        hold_fn_enabled,
        accessibility_trusted,
        runtime_state,
        ready: installed
            && dictation_enabled
            && hold_fn_enabled
            && accessibility_trusted
            && runtime_state == MetaAppRuntimeState::Ready,
    }
}

pub fn open_meta_ai() -> Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", META_AI_BUNDLE_ID])
        .status()
        .context("failed to open Meta AI")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("macOS could not open Meta AI"))
    }
}

fn read_meta_default(key: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", META_AI_BUNDLE_ID, key])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_default_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

mod macos {
    use super::{
        classify_runtime_state, non_pill_window_is_visible, target_still_focused,
        MetaAppRuntimeState, MetaUiSnapshot, TargetApplication, META_AI_BUNDLE_ID,
    };
    use anyhow::{anyhow, Result};
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    use objc2_foundation::NSString;
    use std::ffi::c_void;
    use std::os::raw::c_char;
    use std::ptr;
    use std::sync::OnceLock;

    const AX_SUCCESS: i32 = 0;
    const AX_VALUE_CGPOINT: u32 = 1;
    const AX_VALUE_CGSIZE: u32 = 2;
    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    type AxUiElementRef = *const c_void;
    type AxValueRef = *const c_void;
    type CfArrayRef = *const c_void;
    type CfStringRef = *const c_void;
    type CfTypeRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct PillObservation {
        pub(super) present: bool,
        pub(super) moved: bool,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateApplication(pid: i32) -> AxUiElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AxUiElementRef,
            attribute: CfStringRef,
            value: *mut CfTypeRef,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: AxUiElementRef,
            attribute: CfStringRef,
            value: CfTypeRef,
        ) -> i32;
        fn AXValueCreate(value_type: u32, value: *const c_void) -> AxValueRef;
        fn AXValueGetValue(value: AxValueRef, value_type: u32, output: *mut c_void) -> bool;

    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CfArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CfArrayRef, index: isize) -> *const c_void;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CfTypeRef) -> bool;
        fn CFEqual(value1: CfTypeRef, value2: CfTypeRef) -> bool;
        fn CFGetTypeID(value: CfTypeRef) -> usize;
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            value: *const c_char,
            encoding: u32,
        ) -> CfStringRef;
        fn CFRelease(value: CfTypeRef);
    }

    fn cached_cf_string(cell: &OnceLock<usize>, value: &'static [u8]) -> CfStringRef {
        let pointer = cell.get_or_init(|| unsafe {
            CFStringCreateWithCString(ptr::null(), value.as_ptr().cast(), CF_STRING_ENCODING_UTF8)
                as usize
        });
        *pointer as CfStringRef
    }

    fn ax_windows_attribute() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXWindows\0")
    }

    fn ax_size_attribute() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXSize\0")
    }

    fn ax_position_attribute() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXPosition\0")
    }

    fn ax_subrole_attribute() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXSubrole\0")
    }

    fn ax_minimized_attribute() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXMinimized\0")
    }

    fn ax_dialog_subrole() -> CfStringRef {
        static VALUE: OnceLock<usize> = OnceLock::new();
        cached_cf_string(&VALUE, b"AXDialog\0")
    }

    pub(super) fn is_installed() -> bool {
        let workspace = NSWorkspace::sharedWorkspace();
        let bundle_id = NSString::from_str(META_AI_BUNDLE_ID);
        workspace
            .URLForApplicationWithBundleIdentifier(&bundle_id)
            .is_some()
    }

    pub(super) fn accessibility_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub(super) fn runtime_state() -> MetaAppRuntimeState {
        let Some(app) = running_application() else {
            return MetaAppRuntimeState::NotRunning;
        };
        classify_runtime_state(true, meta_is_frontmost(&app), ui_snapshot(&app).ok())
    }

    /// Meta 3.0 forces itself active when launched, even through LaunchServices
    /// background options. Murmur therefore never launches or hides it. The
    /// user closes its main window once, leaving the signed app running in the
    /// menu bar, and this precondition is rechecked throughout dictation.
    pub(super) fn frontmost_target() -> Result<TargetApplication> {
        let frontmost = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or_else(|| anyhow!("Murmur could not identify the app where you want to type"))?;
        let pid = frontmost.processIdentifier();
        let meta_pid = running_application().map(|app| app.processIdentifier());
        if pid == std::process::id() as i32 || meta_pid == Some(pid) {
            return Err(anyhow!(
                "Switch to the app where you want to type before starting dictation"
            ));
        }
        Ok(TargetApplication { pid })
    }

    pub(super) fn ensure_ready_for_target(
        target: TargetApplication,
        allow_owned_pill: bool,
    ) -> Result<()> {
        let meta = running_application().ok_or_else(|| {
            anyhow!("Meta AI is not running; open it, close its main window, and retry")
        })?;
        if meta_is_frontmost(&meta) {
            return Err(anyhow!(
                "Meta AI is active; return to the app where you want to type"
            ));
        }
        let frontmost_pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| app.processIdentifier());
        if !target_still_focused(target, frontmost_pid) {
            return Err(anyhow!(
                "the app selected for dictation is no longer focused"
            ));
        }
        let snapshot = ui_snapshot(&meta)?;
        if snapshot.non_pill_window_visible {
            return Err(anyhow!(
                "Meta AI has a visible window; close it and return to the app where you want to type"
            ));
        }
        if snapshot.pill_present && !allow_owned_pill {
            return Err(anyhow!(
                "Meta AI already has an active dictation; finish it before starting Murmur"
            ));
        }
        Ok(())
    }

    fn meta_is_frontmost(app: &NSRunningApplication) -> bool {
        app.isActive()
            || NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|frontmost| frontmost.processIdentifier() == app.processIdentifier())
    }

    fn ui_snapshot(app: &NSRunningApplication) -> Result<MetaUiSnapshot> {
        let root = unsafe { AXUIElementCreateApplication(app.processIdentifier()) };
        if root.is_null() {
            return Err(anyhow!("Murmur could not inspect Meta AI windows"));
        }

        let mut windows_value: CfTypeRef = ptr::null();
        let copied = unsafe {
            AXUIElementCopyAttributeValue(root, ax_windows_attribute(), &mut windows_value)
        } == AX_SUCCESS;
        if !copied || windows_value.is_null() {
            unsafe { CFRelease(root) };
            return Err(anyhow!("Murmur could not inspect Meta AI windows"));
        }

        let mut snapshot = MetaUiSnapshot::default();
        let count = unsafe { CFArrayGetCount(windows_value) };
        for index in 0..count {
            let window = unsafe { CFArrayGetValueAtIndex(windows_value, index) };
            if window.is_null() {
                continue;
            }
            if dictation_pill_size(window).is_some() {
                snapshot.pill_present = true;
                continue;
            }
            if non_pill_window_is_visible(copy_bool_attribute(window, ax_minimized_attribute())) {
                snapshot.non_pill_window_visible = true;
            }
        }

        unsafe {
            CFRelease(windows_value);
            CFRelease(root);
        }
        Ok(snapshot)
    }

    fn copy_bool_attribute(element: AxUiElementRef, attribute: CfStringRef) -> Option<bool> {
        let mut value: CfTypeRef = ptr::null();
        let copied =
            unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) } == AX_SUCCESS;
        if !copied || value.is_null() {
            return None;
        }
        let is_boolean = unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() };
        let result = is_boolean.then(|| unsafe { CFBooleanGetValue(value) });
        unsafe { CFRelease(value) };
        result
    }

    pub(super) fn position_dictation_pill(
        overlay_frame: (f64, f64, f64, f64),
    ) -> Result<PillObservation> {
        let app = running_application()
            .ok_or_else(|| anyhow!("Meta AI stopped while Murmur was dictating"))?;
        let pid = app.processIdentifier();
        let root = unsafe { AXUIElementCreateApplication(pid) };
        if root.is_null() {
            return Err(anyhow!("Murmur could not inspect Meta AI's indicator"));
        }

        let mut windows_value: CfTypeRef = ptr::null();
        let copied = unsafe {
            AXUIElementCopyAttributeValue(root, ax_windows_attribute(), &mut windows_value)
        } == AX_SUCCESS;
        if !copied || windows_value.is_null() {
            unsafe { CFRelease(root) };
            return Err(anyhow!("Murmur could not inspect Meta AI's indicator"));
        }

        let mut observation = PillObservation::default();
        let count = unsafe { CFArrayGetCount(windows_value) };
        for index in 0..count {
            let window = unsafe { CFArrayGetValueAtIndex(windows_value, index) };
            let Some(size) = (!window.is_null())
                .then(|| dictation_pill_size(window))
                .flatten()
            else {
                continue;
            };

            observation.present = true;
            let point = CGPoint {
                x: overlay_frame.0 + (overlay_frame.2 - size.width) / 2.0,
                y: overlay_frame.1 + (overlay_frame.3 - size.height) / 2.0,
            };
            let position =
                unsafe { AXValueCreate(AX_VALUE_CGPOINT, (&point as *const CGPoint).cast()) };
            if !position.is_null() {
                observation.moved |= unsafe {
                    AXUIElementSetAttributeValue(window, ax_position_attribute(), position)
                } == AX_SUCCESS;
                unsafe { CFRelease(position) };
            }
        }

        unsafe {
            CFRelease(windows_value);
            CFRelease(root);
        }
        Ok(observation)
    }

    fn dictation_pill_size(window: AxUiElementRef) -> Option<CGSize> {
        let mut subrole_value: CfTypeRef = ptr::null();
        let copied = unsafe {
            AXUIElementCopyAttributeValue(window, ax_subrole_attribute(), &mut subrole_value)
        } == AX_SUCCESS;
        if !copied || subrole_value.is_null() {
            return None;
        }
        let is_dialog = unsafe { CFEqual(subrole_value, ax_dialog_subrole()) };
        unsafe { CFRelease(subrole_value) };
        if !is_dialog {
            return None;
        }

        let mut size_value: CfTypeRef = ptr::null();
        let copied =
            unsafe { AXUIElementCopyAttributeValue(window, ax_size_attribute(), &mut size_value) }
                == AX_SUCCESS;
        if !copied || size_value.is_null() {
            return None;
        }

        let mut size = CGSize::default();
        let read = unsafe {
            AXValueGetValue(
                size_value,
                AX_VALUE_CGSIZE,
                (&mut size as *mut CGSize).cast(),
            )
        };
        unsafe { CFRelease(size_value) };

        (read && (40.0..=120.0).contains(&size.width) && (10.0..=60.0).contains(&size.height))
            .then_some(size)
    }

    fn running_application() -> Option<objc2::rc::Retained<NSRunningApplication>> {
        let bundle_id = NSString::from_str(META_AI_BUNDLE_ID);
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id).firstObject()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn parses_defaults_boolean_values() {
        assert!(parse_default_bool("1\n"));
        assert!(parse_default_bool("TRUE"));
        assert!(!parse_default_bool("0"));
        assert!(!parse_default_bool("false"));
    }

    #[test]
    fn meta_app_errors_have_a_stable_typed_frontend_contract() {
        assert_eq!(MetaAppErrorEvent::NAME, "meta-app-error-event");
        assert_eq!(
            serde_json::to_value(MetaAppErrorEvent {
                code: MetaAppErrorCode::InspectionUnavailable,
            })
            .unwrap(),
            serde_json::json!({ "code": "inspection_unavailable" })
        );
    }

    #[test]
    fn bridge_starts_idle() {
        let bridge = MetaAppBridge::default();
        assert_eq!(*bridge.session.lock().unwrap(), Session::Idle);
    }

    #[test]
    fn runtime_state_requires_an_inactive_windowless_app_without_existing_dictation() {
        let clear = Some(MetaUiSnapshot::default());
        let visible = Some(MetaUiSnapshot {
            non_pill_window_visible: true,
            ..MetaUiSnapshot::default()
        });
        let dictating = Some(MetaUiSnapshot {
            pill_present: true,
            ..MetaUiSnapshot::default()
        });

        assert_eq!(
            classify_runtime_state(false, false, None),
            MetaAppRuntimeState::NotRunning
        );
        assert_eq!(
            classify_runtime_state(true, true, clear),
            MetaAppRuntimeState::Active
        );
        assert_eq!(
            classify_runtime_state(true, false, visible),
            MetaAppRuntimeState::WindowVisible
        );
        assert_eq!(
            classify_runtime_state(true, false, dictating),
            MetaAppRuntimeState::Dictating
        );
        assert_eq!(
            classify_runtime_state(true, false, None),
            MetaAppRuntimeState::InspectionUnavailable
        );
        assert_eq!(
            classify_runtime_state(true, false, clear),
            MetaAppRuntimeState::Ready
        );
    }

    #[test]
    fn only_a_confirmed_minimized_non_pill_window_is_ignored() {
        assert!(!non_pill_window_is_visible(Some(true)));
        assert!(non_pill_window_is_visible(Some(false)));
        assert!(non_pill_window_is_visible(None));
    }

    #[test]
    fn target_focus_must_match_the_captured_application() {
        let target = TargetApplication { pid: 42 };
        assert!(target_still_focused(target, Some(42)));
        assert!(!target_still_focused(target, Some(43)));
        assert!(!target_still_focused(target, None));
    }

    #[test]
    fn stuck_seen_pill_never_finishes_only_because_finalization_timed_out() {
        assert_eq!(
            finalization_decision(true, 0, FINALIZATION_TIMEOUT, false),
            FinalizationDecision::ReportStuckPill
        );
        assert_eq!(
            finalization_decision(true, 0, FINALIZATION_TIMEOUT, true),
            FinalizationDecision::Wait
        );
    }

    #[test]
    fn missing_previously_seen_pill_finishes_finalization() {
        assert_eq!(
            finalization_decision(true, PILL_MISSING_POLLS, Duration::ZERO, false),
            FinalizationDecision::Finish
        );
    }

    #[test]
    fn never_seen_pill_settles_only_after_starting_grace() {
        assert_eq!(
            finalization_decision(
                false,
                0,
                STARTING_SETTLE_TIME - Duration::from_millis(1),
                false
            ),
            FinalizationDecision::Wait
        );
        assert_eq!(
            finalization_decision(false, 0, STARTING_SETTLE_TIME, false),
            FinalizationDecision::Finish
        );
    }

    #[test]
    fn session_allows_only_one_start_generation() {
        let mut session = Session::Idle;
        assert!(session.begin(1));
        assert!(!session.begin(2));
        assert_eq!(session, Session::Starting { generation: 1 });
    }

    #[test]
    fn stopping_during_launch_prevents_fn_press() {
        let mut session = Session::Idle;
        let generation = 1;
        assert!(session.begin(generation));
        let release_called = Cell::new(false);
        assert!(session
            .request_stop(|| {
                release_called.set(true);
                Ok(())
            })
            .unwrap());
        assert!(!release_called.get());

        let press_called = Cell::new(false);
        let compensation_called = Cell::new(false);
        assert!(!session
            .activate(
                generation,
                || {
                    press_called.set(true);
                    Ok(())
                },
                || {
                    compensation_called.set(true);
                    Ok(())
                },
            )
            .unwrap());
        assert!(!press_called.get());
        assert!(!compensation_called.get());
        assert!(matches!(session, Session::Finalizing { .. }));
    }

    #[test]
    fn shutdown_invalidation_prevents_a_delayed_worker_from_pressing_fn() {
        let mut session = Session::Idle;
        let generation = 3;
        assert!(session.begin(generation));
        assert!(session.invalidate_starting_for_shutdown());
        assert_eq!(session, Session::Idle);

        let press_called = Cell::new(false);
        let release_called = Cell::new(false);
        assert!(!session
            .activate(
                generation,
                || {
                    press_called.set(true);
                    Ok(())
                },
                || {
                    release_called.set(true);
                    Ok(())
                },
            )
            .unwrap());
        assert!(!press_called.get());
        assert!(!release_called.get());
    }

    #[test]
    fn failed_fn_release_keeps_recording_retryable() {
        let mut session = Session::Recording { generation: 4 };
        let result = session.request_stop(|| Err(anyhow!("release failed")));
        assert!(result.is_err());
        assert_eq!(session, Session::ReleaseBlocked { generation: 4 });
        assert!(!session.begin(5));

        assert!(session.request_stop(|| Ok(())).unwrap());
        assert!(matches!(session, Session::Finalizing { generation: 4, .. }));
        assert!(session.finish(4));
        assert!(session.begin(5));
    }

    #[test]
    fn failed_fn_down_is_compensated_before_the_session_can_finish() {
        let mut session = Session::Idle;
        assert!(session.begin(7));
        let release_called = Cell::new(false);

        assert!(session
            .activate(
                7,
                || Err(anyhow!("press failed")),
                || {
                    release_called.set(true);
                    Ok(())
                },
            )
            .is_err());

        assert!(release_called.get());
        assert!(matches!(session, Session::Finalizing { generation: 7, .. }));
        assert!(!session.begin(8));
        assert!(session.finish(7));
        assert!(session.begin(8));
    }

    #[test]
    fn failed_fn_down_with_failed_compensation_blocks_new_generations() {
        let mut session = Session::Idle;
        assert!(session.begin(9));

        assert!(session
            .activate(
                9,
                || Err(anyhow!("press failed")),
                || Err(anyhow!("compensating release failed")),
            )
            .is_err());

        assert_eq!(session, Session::ReleaseBlocked { generation: 9 });
        assert!(!session.begin(10));
        assert!(session.request_stop(|| Ok(())).unwrap());
        assert!(session.finish(9));
        assert!(session.begin(10));
    }

    #[test]
    fn only_finalizing_session_can_finish_a_matching_generation() {
        let mut session = Session::Recording { generation: 11 };
        assert!(!session.finish(11));
        assert!(session.fn_may_be_held());

        assert!(session.request_stop(|| Ok(())).unwrap());
        assert!(!session.finish(10));
        assert!(matches!(
            session,
            Session::Finalizing { generation: 11, .. }
        ));
        assert!(session.finish(11));
    }

    #[test]
    fn only_key_holding_states_need_shutdown_release() {
        assert!(!Session::Idle.fn_may_be_held());
        assert!(!Session::Starting { generation: 12 }.fn_may_be_held());
        assert!(!Session::Finalizing {
            generation: 12,
            since: Instant::now(),
            pill_ownership: PillOwnership::Possible,
        }
        .fn_may_be_held());
        assert!(Session::Recording { generation: 12 }.fn_may_be_held());
        assert!(Session::ReleaseBlocked { generation: 12 }.fn_may_be_held());
    }

    #[test]
    fn shutdown_retries_only_for_a_session_that_may_hold_fn() {
        let attempts = Cell::new(0);
        let mut starting = Session::Starting { generation: 13 };
        assert!(!starting
            .release_for_shutdown(3, || {
                attempts.set(attempts.get() + 1);
                Ok(())
            })
            .unwrap());
        assert_eq!(attempts.get(), 0);

        let mut recording = Session::Recording { generation: 14 };
        assert!(recording
            .release_for_shutdown(3, || {
                let next = attempts.get() + 1;
                attempts.set(next);
                if next < 3 {
                    Err(anyhow!("release failed"))
                } else {
                    Ok(())
                }
            })
            .unwrap());
        assert_eq!(attempts.get(), 3);
        assert!(matches!(
            recording,
            Session::Finalizing { generation: 14, .. }
        ));
    }

    #[test]
    fn pill_ownership_starts_only_after_fn_may_have_been_delivered() {
        let mut session = Session::Idle;
        assert!(session.begin(16));
        assert!(!session.pill_may_be_owned());

        assert!(session.request_stop(|| Ok(())).unwrap());
        assert!(!session.pill_may_be_owned());

        let mut session = Session::Idle;
        assert!(session.begin(17));
        assert!(session.activate(17, || Ok(()), || Ok(())).unwrap());
        assert!(session.pill_may_be_owned());
        assert!(session.request_stop(|| Ok(())).unwrap());
        assert!(session.pill_may_be_owned());
    }

    #[test]
    fn failed_fn_down_keeps_possible_indicator_ownership_after_compensation() {
        let mut session = Session::Idle;
        assert!(session.begin(18));

        assert!(session
            .activate(18, || Err(anyhow!("press failed")), || Ok(()))
            .is_err());

        assert!(session.pill_may_be_owned());
    }

    #[test]
    fn failed_shutdown_release_keeps_the_session_retryable() {
        let attempts = Cell::new(0);
        let mut session = Session::Recording { generation: 15 };

        let result = session.release_for_shutdown(SHUTDOWN_RELEASE_ATTEMPTS, || {
            attempts.set(attempts.get() + 1);
            Err(anyhow!("release failed"))
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), SHUTDOWN_RELEASE_ATTEMPTS);
        assert_eq!(session, Session::ReleaseBlocked { generation: 15 });
        assert!(session.request_stop(|| Ok(())).unwrap());
    }
}
