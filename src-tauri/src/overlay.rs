#[cfg(target_os = "macos")]
use crate::input;
use crate::meetings::announce::Batch;
use crate::meetings::calls::PromptKind;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
use crate::tray_i18n::get_tray_translations;
use crate::ProcessingOperation;
#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSEventMask;
#[cfg(target_os = "macos")]
use std::cell::RefCell;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

use tauri::WebviewUrl;

#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
};

pub mod meeting_notice;
use meeting_notice::MeetingNews;
pub use meeting_notice::{announce_meeting, meeting_updated, settings_meeting_changed};

mod call_prompt;
mod notices;
mod placement;
mod recording;

use call_prompt::*;
pub use call_prompt::{hide_call_prompt, show_call_prompt};
pub(crate) use notices::copy_held_notice;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use notices::hold_copy_notice;
#[cfg(target_os = "macos")]
pub use notices::show_copy_notice;
use notices::*;
pub use notices::{show_cancelled_notice, show_error_overlay, show_mic_lost_notice};
use placement::*;
pub use recording::create_recording_overlay;

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(RecordingOverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

// Native overlay window sizes (logical points), mirroring the `--ov-*` vars in
// RecordingOverlay.css. One window is reused for every state and resized in
// `present_overlay_window`. The capsule sits CARD_MARGIN inside the window edge
// that faces the screen edge, leaving room for its soft shadow; the window
// ignores the mouse except while a fitted pill shows, so that margin never
// blocks clicks.
const CARD_MARGIN: f64 = 16.0;
// Dictation: the 34-point orb. Its size never changes.
const ORB_OVERLAY_SIZE: f64 = 34.0 + 2.0 * CARD_MARGIN;
// Meeting pill: 108 wide and 34 high, the least size of a fitted pill's
// window too. The width stays even so the centred capsule's edges sit on
// whole points.
const MEETING_OVERLAY_WIDTH: f64 = 108.0 + 2.0 * CARD_MARGIN;
const MEETING_OVERLAY_HEIGHT: f64 = 34.0 + 2.0 * CARD_MARGIN;
// Error capsule: at most 304 wide (the orb, then the text) and four lines of
// text, so the longest message fits in every locale whatever the fallback
// font.
const ERROR_OVERLAY_WIDTH: f64 = 304.0 + 2.0 * CARD_MARGIN;
const ERROR_OVERLAY_HEIGHT: f64 = 84.0 + 2.0 * CARD_MARGIN;
const ERROR_DISPLAY_TIME: Duration = Duration::from_secs(4);
// Copy capsule: its note wraps, so only the webview knows its size; it
// measures the capsule and `fit_copy_notice` fits the window to it. Until
// then the window opens at the widest capsule with three lines of note.
const COPY_OVERLAY_WIDTH: f64 = 560.0 + 2.0 * CARD_MARGIN;
const COPY_OVERLAY_HEIGHT: f64 = 62.0 + 2.0 * CARD_MARGIN;
const COPY_DISPLAY_TIME: Duration = Duration::from_secs(8);
const COPY_HOVER_RECHECK: Duration = Duration::from_secs(1);
const COPIED_DISPLAY_TIME: Duration = Duration::from_millis(1500);
#[cfg(target_os = "macos")]
const ESCAPE_KEY_CODE: u16 = 53;

/// Bumped by every show or hide request, so a delayed hide only applies when
/// nothing replaced the overlay in the meantime.
static OVERLAY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// A message that outlives the dictation that raised it. Routine hides
/// (pipeline finished, cancel) leave it up; only its own timer, its dismissal
/// or a new recording replaces it, and the meeting pill returns after it.
enum Notice {
    Error,
    /// No text field had the focus; `write` puts the text on the clipboard
    /// only if the user clicks Copy. With `mic_lost_next`, the microphone
    /// went away too, and its warning shows once this pill goes: one
    /// message at a time.
    Copy {
        text: String,
        write: ClipboardWriter,
        mic_lost_next: bool,
    },
    /// The microphone went away during the dictation, so its text ends
    /// early.
    MicLost,
    /// A dictation was cancelled; its Restore button brings it back.
    Cancelled,
    MeetingNews(MeetingNews),
}

impl Notice {
    /// The pills the webview sizes to their text and Escape dismisses.
    fn is_fitted(&self) -> bool {
        matches!(
            self,
            Notice::Copy { .. } | Notice::MicLost | Notice::Cancelled | Notice::MeetingNews(_)
        )
    }
}

/// Puts a copy notice's text on the clipboard.
pub(crate) type ClipboardWriter = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
#[derive(Default)]
struct Notices {
    current: Option<Notice>,
    pending_meetings: Option<Batch>,
    page_epoch: u64,
    meeting_listener_ready: bool,
}

static NOTICE: Mutex<Notices> = Mutex::new(Notices {
    current: None,
    pending_meetings: None,
    page_epoch: 0,
    meeting_listener_ready: false,
});

fn notice() -> MutexGuard<'static, Notices> {
    NOTICE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn notice_showing() -> bool {
    notice().current.is_some()
}

// Paths are spelled out because `tauri_panel!` imports these names itself.
#[cfg(target_os = "macos")]
type EscapeMonitor = objc2::rc::Retained<objc2::runtime::AnyObject>;

#[cfg(target_os = "macos")]
thread_local! {
    /// The fitted pills' Escape monitors, on the main thread.
    static ESCAPE_MONITORS: RefCell<Vec<EscapeMonitor>> = const { RefCell::new(Vec::new()) };
}

/// Payload of `show-overlay-copy`.
#[derive(Clone, serde::Serialize)]
struct CopyNoticeLabels {
    copy: String,
    copied: String,
    /// Says there was no text field to paste into.
    note: String,
}

/// Payload of `show-overlay-cancelled`.
#[derive(Clone, serde::Serialize)]
struct CancelledNoticeLabels {
    note: String,
    restore: String,
}

const CANCELLED_DISPLAY_TIME: Duration = Duration::from_secs(5);

/// The call prompt's own panel, beside the overlay.
const CALL_PROMPT_WINDOW: &str = "call_prompt";
/// The call prompt sits this much further from the screen edge than the
/// overlay, clear of its tallest capsule, a four-line error: a dictation
/// shows beside it.
const CALL_PROMPT_OFFSET: f64 = 84.0 + 8.0;

/// Payload of `show-overlay-call`, and what `get_call_prompt` answers.
#[derive(Clone, serde::Serialize, specta::Type)]
pub struct CallPromptLabels {
    /// Which answers the buttons send.
    kind: PromptKind,
    question: String,
    /// The filled button.
    accept: String,
    decline: String,
    /// The overlay's side of the screen, which the prompt sits on too.
    position: OverlayPosition,
}

/// The call prompt showing. Its page asks for it once it listens, so a
/// prompt shown while the page still loaded is not lost.
static CALL_PROMPT: Mutex<Option<CallPromptLabels>> = Mutex::new(None);

fn call_prompt() -> MutexGuard<'static, Option<CallPromptLabels>> {
    CALL_PROMPT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a dictation owns the overlay (recording or transcribing).
static DICTATION_SHOWING: AtomicBool = AtomicBool::new(false);
/// Counts the dictation states that took the overlay. `DICTATION_SHOWING`
/// stays set for a cancelled dictation until the coordinator hides its
/// overlay, so only this count tells that dictation from a newer one.
static DICTATION_EPOCH: AtomicU64 = AtomicU64::new(0);

/// How many dictation states had taken the overlay at some moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictationEpoch(u64);

/// The epoch now. A dictation reads it once its own state has shown, and a
/// pill raised for it later is dropped when the epoch has moved on.
pub fn dictation_epoch() -> DictationEpoch {
    DictationEpoch(DICTATION_EPOCH.load(Ordering::SeqCst))
}

/// The meeting pill, while a meeting records. Dictation and error capsules
/// take the overlay over for a moment; hiding them brings the pill back.
static MEETING_PILL: Mutex<Option<MeetingPill>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct MeetingPill {
    pub started_at_ms: i64,
    pub stops_at_ms: i64,
}

fn meeting_pill() -> Option<MeetingPill> {
    *MEETING_PILL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static LAST_MIC_LEVEL_EMIT: AtomicU64 = AtomicU64::new(0);
const EMIT_THROTTLE_MS: u64 = 33; // ~30 FPS

// Distance from the screen edge to the visible capsule.
const OVERLAY_TOP_OFFSET: f64 = 46.0;
const OVERLAY_BOTTOM_OFFSET: f64 = 16.0;

fn show_overlay_state(app_handle: &AppHandle, state: &str) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }

    // The flag is set under the notice lock, which the cancelled pill reads it
    // under: it shows before this state, which replaces it, or not at all.
    let mut held = notice();
    if !dictation_takes_overlay(&mut held.current, state) {
        log::debug!("Leaving the notice up over a late '{state}' overlay");
        return;
    }
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    DICTATION_EPOCH.fetch_add(1, Ordering::SeqCst);
    DICTATION_SHOWING.store(true, Ordering::SeqCst);
    drop(held);
    let handle = app_handle.clone();
    let state = state.to_string();
    let _ = app_handle.run_on_main_thread(move || {
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) =
            present_overlay_window(&handle, &state, ORB_OVERLAY_SIZE, ORB_OVERLAY_SIZE, false)
        {
            let _ = window.emit("show-overlay", state);
        }
    });
}

/// Whether a dictation showing `state` takes the overlay from `notice`,
/// dropping it. Only a new recording replaces a fitted pill: a transcribing
/// state that comes after one belongs to a dictation already over, and
/// would leave the copy pill's Copy with nothing to copy. Restore replaces
/// the cancelled pill or meeting news directly, without a recording state.
fn dictation_takes_overlay(notice: &mut Option<Notice>, state: &str) -> bool {
    if state != "recording"
        && notice.as_ref().is_some_and(|notice| {
            notice.is_fitted() && !matches!(notice, Notice::Cancelled | Notice::MeetingNews(_))
        })
    {
        return false;
    }
    *notice = None;
    true
}

/// The call prompt showing, if any.
#[tauri::command]
#[specta::specta]
pub fn get_call_prompt() -> Option<CallPromptLabels> {
    call_prompt().clone()
}

/// Fits the call prompt's window to the capsule the webview measured, as
/// `fit_copy_notice` does the copy notice's.
#[tauri::command]
#[specta::specta]
pub fn fit_call_prompt(app: AppHandle, width: f64, height: f64) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(CALL_PROMPT_WINDOW) {
            let (width, height) = fitted_window_size(&handle, width, height);
            place_call_prompt(&handle, &window, width, height);
        }
    });
}

/// Fits a fitted pill's window, the copy pill's or the microphone
/// warning's, to the capsule the webview measured, so a note that wraps
/// further in a wider fallback font still shows whole, and the window, which
/// takes clicks while the pill shows, stays close to it.
#[tauri::command]
#[specta::specta]
pub fn fit_copy_notice(app: AppHandle, width: f64, height: f64) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !notice().current.as_ref().is_some_and(Notice::is_fitted) {
            return;
        }
        let Some(window) = handle.get_webview_window("recording_overlay") else {
            return;
        };
        let (width, height) = fitted_window_size(&handle, width, height);
        #[cfg(target_os = "windows")]
        place_overlay(&handle, &window, width, height, 0.0);
        #[cfg(target_os = "macos")]
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
        #[cfg(target_os = "macos")]
        if let Some((x, y)) = calculate_overlay_position(&handle, width, height) {
            let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    });
}

/// `copy_window_size` on the screen under the cursor.
fn fitted_window_size(app_handle: &AppHandle, width: f64, height: f64) -> (f64, f64) {
    let work_area = get_monitor_with_cursor(app_handle).map(|monitor| {
        let scale = monitor.scale_factor();
        let area = monitor.work_area().size;
        (area.width as f64 / scale, area.height as f64 / scale)
    });
    copy_window_size(
        (width, height),
        work_area.unwrap_or((f64::INFINITY, f64::INFINITY)),
    )
}

/// The copy pill's click: puts its text on the clipboard, then lets the pill
/// read "Copied" for a moment before it hides.
#[tauri::command]
#[specta::specta]
pub async fn copy_notice_again(app: AppHandle) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(copy_held_notice());
    })
    .map_err(|error| format!("Cannot reach the clipboard: {error}"))?;
    receiver
        .await
        .map_err(|_| "The clipboard write did not run".to_string())??;
    // Its own timer may have ended the pill a moment before: the text is
    // copied, but the warning that followed the pill keeps its time, with no
    // "Copied".
    let generation = {
        let held = notice();
        if !copy_pill_showing(&held.current) {
            return Ok(());
        }
        OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    };
    dismiss_notice_after(&app, generation, COPIED_DISPLAY_TIME, false);
    Ok(())
}

/// While a fitted pill shows, the overlay takes clicks; at any other time
/// the mouse goes through it to the app below.
#[cfg(target_os = "windows")]
fn set_interactive(
    _app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    interactive: bool,
) {
    let _ = window.set_ignore_cursor_events(!interactive);
}

/// While a fitted pill shows, the overlay takes clicks and Escape; at any
/// other time the mouse goes through it to the app below. Main thread only.
#[cfg(target_os = "macos")]
fn set_interactive(
    app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    interactive: bool,
) {
    let _ = window.set_ignore_cursor_events(!interactive);
    ESCAPE_MONITORS.with(|slot| {
        let mut monitors = slot.borrow_mut();
        if interactive && monitors.is_empty() {
            *monitors = add_escape_monitors(app_handle);
        } else if !interactive {
            for monitor in monitors.drain(..) {
                unsafe { objc2_app_kit::NSEvent::removeMonitor(&monitor) };
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn escape_dismissal(key_code: u16) -> Option<Dismissal> {
    (key_code == ESCAPE_KEY_CODE).then_some(Dismissal::Escape)
}

#[cfg(target_os = "macos")]
fn dismiss_escape_key(app_handle: &AppHandle, key_code: u16) {
    if let Some(dismissal) = escape_dismissal(key_code) {
        // Dismissing removes the monitors, so it runs outside their handlers.
        let app = app_handle.clone();
        std::thread::spawn(move || dismiss_notice(&app, dismissal));
    }
}

/// Global monitors exclude Murmur's own events. The local monitor leaves those
/// events unchanged so Escape still reaches the focused Settings control.
#[cfg(target_os = "macos")]
fn add_escape_monitors(app_handle: &AppHandle) -> Vec<EscapeMonitor> {
    let app = app_handle.clone();
    let global_handler = RcBlock::new(move |event: NonNull<objc2_app_kit::NSEvent>| {
        dismiss_escape_key(&app, unsafe { event.as_ref() }.keyCode());
    });
    let global = objc2_app_kit::NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::KeyDown,
        &global_handler,
    );
    let app = app_handle.clone();
    let local_handler = RcBlock::new(move |event: NonNull<objc2_app_kit::NSEvent>| {
        dismiss_escape_key(&app, unsafe { event.as_ref() }.keyCode());
        event.as_ptr()
    });
    let local = unsafe {
        objc2_app_kit::NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown,
            &local_handler,
        )
    };
    [global, local].into_iter().flatten().collect()
}

#[cfg(target_os = "windows")]
fn present_overlay_window(
    app_handle: &AppHandle,
    state: &str,
    width: f64,
    height: f64,
    interactive: bool,
) -> Option<tauri::webview::WebviewWindow> {
    let overlay_window = app_handle.get_webview_window("recording_overlay")?;
    let started = std::time::Instant::now();
    place_overlay(app_handle, &overlay_window, width, height, 0.0);
    let _ = overlay_window.show();
    // Showing can move a window that changed monitor back by tao's stale
    // scale; placing it again settles it.
    place_overlay(app_handle, &overlay_window, width, height, 0.0);
    set_interactive(app_handle, &overlay_window, interactive);
    log::debug!("overlay '{}': place+show={:?}", state, started.elapsed());
    Some(overlay_window)
}

#[cfg(target_os = "macos")]
fn present_overlay_window(
    app_handle: &AppHandle,
    state: &str,
    width: f64,
    height: f64,
    interactive: bool,
) -> Option<tauri::webview::WebviewWindow> {
    let overlay_window = app_handle.get_webview_window("recording_overlay")?;
    let size_started = std::time::Instant::now();
    let _ = overlay_window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
    let size_elapsed = size_started.elapsed();

    let pos_started = std::time::Instant::now();
    let set_pos_elapsed =
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let set_pos_started = std::time::Instant::now();
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
            set_pos_started.elapsed()
        } else {
            std::time::Duration::ZERO
        };
    let pos_calc_elapsed = pos_started.elapsed() - set_pos_elapsed;

    let show_started = std::time::Instant::now();
    let _ = overlay_window.show();
    let show_elapsed = show_started.elapsed();
    set_interactive(app_handle, &overlay_window, interactive);

    log::debug!(
        "overlay '{}': set_size={:?} pos_calc={:?} set_pos={:?} show={:?}",
        state,
        size_elapsed,
        pos_calc_elapsed,
        set_pos_elapsed,
        show_elapsed
    );
    Some(overlay_window)
}

/// Notify the visible recording overlay that the input stream has delivered its
/// first sample chunk. Audio feedback uses the same backend readiness signal,
/// but this targeted event is skipped when overlays are disabled.
pub fn emit_recording_ready(app_handle: &AppHandle) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let _ = handle.emit_to("recording_overlay", "recording-ready", ());
    });
}

/// Shows the recording overlay window with fade-in animation
pub fn show_recording_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "recording");
}

/// Shows the transcribing overlay window
pub fn show_transcribing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "transcribing");
}

/// Updates the overlay window position based on current settings
pub fn update_overlay_position(app_handle: &AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || update_overlay_position_on_main(&handle));
}

/// Hides the recording overlay window with fade-out animation, or brings the
/// meeting pill back while a meeting records.
pub fn hide_recording_overlay(app_handle: &AppHandle) {
    let held = notice();
    if held.current.is_some() {
        return;
    }
    DICTATION_SHOWING.store(false, Ordering::SeqCst);
    drop(held);
    settle_overlay(app_handle);
}

/// Leaves the overlay as it is when no dictation or notice holds it: the
/// meeting pill, or hidden, so a prompt that held it never stays behind.
fn settle_overlay(app_handle: &AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let style = settings::get_settings(&handle).overlay_style;
        let mut held = notice();
        if held.current.is_some() || DICTATION_SHOWING.load(Ordering::SeqCst) {
            return;
        }
        if meeting_notice::show_pending(&handle, &mut held, style) {
            return;
        }
        match resting_pill(meeting_pill(), style) {
            Some(pill) => present_meeting_pill(&handle, pill),
            None => fade_out_overlay(&handle),
        }
    });
}

/// The meeting pill shows while a meeting records, unless the overlay is
/// off.
fn resting_pill(pill: Option<MeetingPill>, style: OverlayStyle) -> Option<MeetingPill> {
    pill.filter(|_| style != OverlayStyle::None)
}

/// Shows the meeting pill unless a dictation or a notice is on screen; it
/// appears when they hide.
pub fn show_meeting_overlay(app_handle: &AppHandle, started_at_ms: i64, stops_at_ms: i64) {
    let pill = MeetingPill {
        started_at_ms,
        stops_at_ms,
    };
    *MEETING_PILL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pill);
    if !DICTATION_SHOWING.load(Ordering::SeqCst) && !notice_showing() {
        settle_overlay(app_handle);
    }
}

pub fn hide_meeting_overlay(app_handle: &AppHandle) {
    let was_showing = MEETING_PILL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .is_some();
    if was_showing && !DICTATION_SHOWING.load(Ordering::SeqCst) && !notice_showing() {
        settle_overlay(app_handle);
    }
}

fn present_meeting_pill(app_handle: &AppHandle, pill: MeetingPill) {
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = present_overlay_window(
            &handle,
            "meeting",
            MEETING_OVERLAY_WIDTH,
            MEETING_OVERLAY_HEIGHT,
            false,
        ) {
            let _ = window.emit("show-overlay-meeting", pill);
        }
    });
}

fn fade_out_overlay(app_handle: &AppHandle) {
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        let handle = app_handle.clone();
        let window = overlay_window.clone();
        let _ = app_handle.run_on_main_thread(move || set_interactive(&handle, &window, false));
        let _ = overlay_window.emit("hide-overlay", ());
        let window_clone = overlay_window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if OVERLAY_GENERATION.load(Ordering::SeqCst) == generation {
                let _ = window_clone.hide();
            }
        });
    }
}

static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);

/// Update the cached overlay-enabled flag. Called from `lib.rs` at
/// startup after settings load, and from `change_overlay_style_setting`
/// whenever the user changes whether the overlay is shown.
pub fn update_overlay_enabled_cache(enabled: bool) {
    OVERLAY_ENABLED.store(enabled, Ordering::Relaxed);
    if !enabled {
        let mut held = notice();
        held.pending_meetings = None;
        if matches!(held.current, Some(Notice::MeetingNews(_))) {
            held.current = None;
            OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub fn emit_levels(app_handle: &AppHandle, levels: &[f32]) {
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = LAST_MIC_LEVEL_EMIT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < EMIT_THROTTLE_MS {
        return;
    }
    LAST_MIC_LEVEL_EMIT.store(now, Ordering::Relaxed);

    let _ = app_handle.emit_to("recording_overlay", "mic-level", levels);
}

#[cfg(test)]
mod tests;
