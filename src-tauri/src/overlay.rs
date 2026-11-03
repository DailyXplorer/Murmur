use crate::input;
use crate::meetings::announce::Batch;
use crate::meetings::calls::PromptKind;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
use crate::tray_i18n::get_tray_translations;
use crate::ProcessingOperation;
use block2::RcBlock;
use objc2_app_kit::NSEventMask;
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

use tauri::WebviewUrl;

use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
};

pub mod meeting_notice;
use meeting_notice::MeetingNews;
pub use meeting_notice::{announce_meeting, meeting_updated, settings_meeting_changed};

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
    meeting_listener_ready: bool,
}

static NOTICE: Mutex<Notices> = Mutex::new(Notices {
    current: None,
    pending_meetings: None,
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
type EscapeMonitor = objc2::rc::Retained<objc2::runtime::AnyObject>;

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

fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    if let Some(mouse_location) = input::get_cursor_position(app_handle) {
        if let Ok(monitors) = app_handle.available_monitors() {
            for monitor in monitors {
                let scale = monitor.scale_factor();
                let pos = PhysicalPosition::new(
                    (monitor.position().x as f64 / scale) as i32,
                    (monitor.position().y as f64 / scale) as i32,
                );
                let size = PhysicalSize::new(
                    (monitor.size().width as f64 / scale) as u32,
                    (monitor.size().height as f64 / scale) as u32,
                );
                if is_point_within(mouse_location, &pos, &size) {
                    return Some(monitor);
                }
            }
        }
    }

    app_handle.primary_monitor().ok().flatten()
}

fn is_point_within(
    point: (i32, i32),
    origin: &PhysicalPosition<i32>,
    size: &PhysicalSize<u32>,
) -> bool {
    let (x, y) = point;
    x >= origin.x
        && x < origin.x + size.width as i32
        && y >= origin.y
        && y < origin.y + size.height as i32
}

/// The panel never becomes key, so its webview gets no hover events; the
/// pointer is read from the window server instead. Compared in points: tao
/// scales the cursor by the primary display and the window by its own.
fn pointer_over_overlay(app_handle: &AppHandle) -> bool {
    let Some(window) = app_handle.get_webview_window("recording_overlay") else {
        return false;
    };
    let Some(pointer) = input::get_cursor_position(app_handle) else {
        return false;
    };
    match (
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
    ) {
        (Ok(origin), Ok(size), Ok(scale)) => is_point_within(
            pointer,
            &PhysicalPosition::new(
                (origin.x as f64 / scale) as i32,
                (origin.y as f64 / scale) as i32,
            ),
            &PhysicalSize::new(
                (size.width as f64 / scale) as u32,
                (size.height as f64 / scale) as u32,
            ),
        ),
        _ => false,
    }
}

/// Returns overlay position in logical coordinates (points on macOS).
///
/// The Bottom anchor uses the macOS work area (visibleFrame) so the overlay
/// tracks the Dock: above it when shown, at the screen edge when hidden.
/// This relies on the work_area.position.y fix in tauri 2.11; earlier
/// versions reported a wrong work area origin. Top keeps full monitor bounds
/// plus the fixed offset.
///
/// We must use LogicalPosition (not PhysicalPosition) because Tauri/tao
/// converts PhysicalPosition using the scale factor of the monitor the window
/// is *currently* on, which is wrong when moving cross-monitor.
fn calculate_overlay_position(
    app_handle: &AppHandle,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let monitor = get_monitor_with_cursor(app_handle)?;
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;

    let settings = settings::get_settings(app_handle);

    let x = monitor_x + (monitor_width - width) / 2.0;
    let y = match settings.overlay_position {
        OverlayPosition::Top => monitor_y + OVERLAY_TOP_OFFSET - CARD_MARGIN,
        OverlayPosition::Bottom => {
            let wa = monitor.work_area();
            let bottom = (wa.position.y as f64 + wa.size.height as f64) / scale;
            bottom - height - (OVERLAY_BOTTOM_OFFSET - CARD_MARGIN)
        }
    };

    Some((x, y))
}

fn current_overlay_logical_size(window: &tauri::webview::WebviewWindow) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

/// Creates the recording overlay panel and keeps it hidden by default.
pub fn create_recording_overlay(app_handle: &AppHandle) {
    meeting_notice::listen_for_page(app_handle);
    match PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, "recording_overlay")
        .url(WebviewUrl::App("src/overlay/index.html".into()))
        .title("Recording")
        .level(PanelLevel::Status)
        .size(tauri::Size::Logical(tauri::LogicalSize {
            width: MEETING_OVERLAY_WIDTH,
            height: MEETING_OVERLAY_HEIGHT,
        }))
        .has_shadow(false)
        .transparent(true)
        .no_activate(true)
        .corner_radius(0.0)
        .style_mask(StyleMask::empty().borderless().nonactivating_panel())
        // The panel never becomes key, so without first-mouse the copy
        // notice's first click would be swallowed.
        .with_window(|w| {
            w.decorations(false)
                .transparent(true)
                .focusable(false)
                .accept_first_mouse(true)
        })
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary(),
        )
        .build()
    {
        Ok(panel) => {
            panel.hide();
            if let Some(window) = app_handle.get_webview_window("recording_overlay") {
                let _ = window.set_ignore_cursor_events(true);
            }
        }
        Err(e) => {
            log::error!("Failed to create recording overlay panel: {}", e);
        }
    }
}

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

/// Shows a short error message in the overlay for a few seconds. Returns
/// false when the overlay is disabled, so the caller can fall back to a
/// notification.
pub fn show_error_overlay(app_handle: &AppHandle, message: String) -> bool {
    if settings::get_settings(app_handle).overlay_style == OverlayStyle::None {
        return false;
    }
    let generation = {
        let mut held = notice();
        held.current = Some(Notice::Error);
        OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
    };
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) = present_overlay_window(
            &handle,
            "error",
            ERROR_OVERLAY_WIDTH,
            ERROR_OVERLAY_HEIGHT,
            false,
        ) {
            let _ = window.emit("show-overlay-error", message);
        }
    });
    dismiss_notice_after(app_handle, generation, ERROR_DISPLAY_TIME, false);
    true
}

/// Keeps `text` as the copy notice's, for its Copy click to hand to `write`.
/// With `mic_lost`, the microphone warning follows the pill.
pub(crate) fn hold_copy_notice(text: String, write: ClipboardWriter, mic_lost: bool) -> u64 {
    let mut held = notice();
    held.current = Some(Notice::Copy {
        text,
        write,
        mic_lost_next: mic_lost,
    });
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Shows the copy pill, a short note and a "Copy" action, because no text
/// field had the focus. A click hands the text to `write`, Escape dismisses
/// it. With `mic_lost`, the microphone warning shows once the pill goes, so
/// the two never share a pill. Returns false when the overlay is disabled,
/// so there is no pill to click.
pub fn show_copy_notice(
    app_handle: &AppHandle,
    text: String,
    write: ClipboardWriter,
    mic_lost: bool,
) -> bool {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return false;
    }
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CopyNoticeLabels {
        copy: strings.overlay_copy,
        copied: strings.overlay_copied,
        note: strings.overlay_not_pasted,
    };
    let generation = hold_copy_notice(text, write, mic_lost);
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Some(window) = present_overlay_window(
            &handle,
            "copy",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-copy", labels);
        }
    });
    dismiss_notice_after(app_handle, generation, COPY_DISPLAY_TIME, true);
    true
}

/// Holds the microphone warning as `operation`'s notice, unless the copy
/// pill shows: the warning follows it already, and its Copy must keep its
/// text. A cancelled operation holds nothing: a newer dictation may own the
/// overlay since its cancel.
fn hold_mic_lost_notice(notice: &mut Option<Notice>, operation: &ProcessingOperation) -> bool {
    if matches!(notice, Some(Notice::Copy { .. })) || operation.is_cancelled() {
        return false;
    }
    *notice = Some(Notice::MicLost);
    true
}

/// Says the microphone went away during `operation`'s dictation, in a pill
/// shown and dismissed like the copy pill, without its action. Nothing shows
/// when the overlay is disabled or the operation was cancelled.
pub fn show_mic_lost_notice(app_handle: &AppHandle, operation: &ProcessingOperation) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }
    // Held until the pill is queued, so a recording that starts after the
    // cancel check queues its own state after it, with a later generation.
    let mut held = notice();
    if !hold_mic_lost_notice(&mut held.current, operation) {
        return;
    }
    present_mic_lost_notice(app_handle, held);
}

/// Shows the microphone warning `held` holds, and lets the lock go once the
/// pill is queued.
fn present_mic_lost_notice(app_handle: &AppHandle, held: MutexGuard<'static, Notices>) {
    let settings = settings::get_settings(app_handle);
    let message = get_tray_translations(Some(settings.app_language)).overlay_mic_disconnected;
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = present_overlay_window(
            &handle,
            "mic-lost",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-warning", message);
        }
    });
    drop(held);
    dismiss_notice_after(app_handle, generation, COPY_DISPLAY_TIME, true);
}

/// Holds the cancelled pill as the notice, unless a newer dictation took the
/// overlay since the cancelled one showed at `shown`. The pill comes from a
/// pipeline that unwinds after the cancel, so a newer dictation may have
/// shown its own since: its recorder is not open yet, or it already
/// transcribes, and the cancelled pill would cover it. The cancelled dictation
/// itself may still own the overlay, until the coordinator hides it, and keeps
/// its pill.
fn hold_cancelled_notice(
    notice: &mut Option<Notice>,
    shown: DictationEpoch,
    now: DictationEpoch,
) -> bool {
    if now != shown {
        return false;
    }
    *notice = Some(Notice::Cancelled);
    true
}

/// Says the dictation that showed at `shown` was cancelled, with a Restore
/// button, in a pill shown and dismissed like the copy pill. Nothing shows
/// when the overlay is disabled or a newer dictation shows: the tray item
/// still restores it.
pub fn show_cancelled_notice(app_handle: &AppHandle, shown: DictationEpoch) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CancelledNoticeLabels {
        note: strings.overlay_cancelled,
        restore: strings.overlay_restore,
    };
    // Held until the pill is queued, so a dictation that shows after the
    // check queues its own state after it, with a later generation.
    let mut held = notice();
    if !hold_cancelled_notice(&mut held.current, shown, dictation_epoch()) {
        log::debug!("Leaving the new dictation's overlay over a late cancelled notice");
        return;
    }
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) = present_overlay_window(
            &handle,
            "cancelled",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        ) {
            let _ = window.emit("show-overlay-cancelled", labels);
        }
    });
    drop(held);
    dismiss_notice_after(app_handle, generation, CANCELLED_DISPLAY_TIME, true);
}

/// Creates the call prompt's panel, hidden. It takes clicks whenever it
/// shows. It only lives while a prompt shows.
fn create_call_prompt(app_handle: &AppHandle) {
    if let Err(e) = PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, CALL_PROMPT_WINDOW)
        .url(WebviewUrl::App("src/overlay/call.html".into()))
        .title("Call")
        .level(PanelLevel::Status)
        .size(tauri::Size::Logical(tauri::LogicalSize {
            width: COPY_OVERLAY_WIDTH,
            height: COPY_OVERLAY_HEIGHT,
        }))
        .has_shadow(false)
        .transparent(true)
        .no_activate(true)
        .corner_radius(0.0)
        .style_mask(StyleMask::empty().borderless().nonactivating_panel())
        .with_window(|w| {
            w.decorations(false)
                .transparent(true)
                .focusable(false)
                .accept_first_mouse(true)
        })
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary(),
        )
        .build()
        .map(|panel| panel.hide())
    {
        log::error!("Failed to create the call prompt panel: {}", e);
    }
}

/// Asks whether to record the call that started, with "Record" and "Not
/// now", or, once the call a meeting records ended, whether to stop, with
/// "Stop and transcribe" and "Keep recording", in a panel of its own: a
/// dictation, an error or the meeting pill never hides it. It shows even
/// with the overlay off: the call prompt has a setting of its own. The
/// panel never takes the focus from the call.
pub fn show_call_prompt(app_handle: &AppHandle, kind: PromptKind) {
    let settings = settings::get_settings(app_handle);
    let strings = get_tray_translations(Some(settings.app_language));
    let (question, accept, decline) = match kind {
        PromptKind::Start => (
            strings.call_record_question,
            strings.call_record,
            strings.call_not_now,
        ),
        PromptKind::Ended => (
            strings.call_ended_question,
            strings.call_ended_stop,
            strings.call_ended_keep,
        ),
    };
    let labels = CallPromptLabels {
        kind,
        question,
        accept,
        decline,
        position: settings.overlay_position,
    };
    *call_prompt() = Some(labels.clone());
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        // A page built here asks for the labels once it listens.
        if handle.get_webview_window(CALL_PROMPT_WINDOW).is_none() {
            create_call_prompt(&handle);
        }
        let Some(window) = handle.get_webview_window(CALL_PROMPT_WINDOW) else {
            return;
        };
        place_call_prompt(&handle, &window, COPY_OVERLAY_WIDTH, COPY_OVERLAY_HEIGHT);
        let _ = window.show();
        let _ = handle.emit_to(CALL_PROMPT_WINDOW, "show-overlay-call", labels);
    });
}

pub fn hide_call_prompt(app_handle: &AppHandle) {
    *call_prompt() = None;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        // The panel's window must get its own class back before Tauri
        // destroys it: destroying it as a panel raises an Objective-C
        // exception, which aborts Murmur. `to_window` also takes the panel
        // out of the store, or its web process would stay. It marks the
        // window released when closed, but tao releases its windows itself:
        // left on, the window would be freed twice.
        let window = match handle.get_webview_panel(CALL_PROMPT_WINDOW) {
            Ok(panel) => {
                panel.hide();
                let window = panel.to_window();
                panel.set_released_when_closed(false);
                window
            }
            // Already handed back by an earlier hide, its destroy pending.
            Err(_) => handle.get_webview_window(CALL_PROMPT_WINDOW),
        };
        if let Some(window) = window {
            let _ = window.hide();
            if let Err(e) = window.destroy() {
                log::error!("Failed to destroy the call prompt panel: {}", e);
            }
        }
    });
}

fn place_call_prompt(
    app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    width: f64,
    height: f64,
) {
    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
    if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
        let y = match settings::get_settings(app_handle).overlay_position {
            OverlayPosition::Top => y + CALL_PROMPT_OFFSET,
            OverlayPosition::Bottom => y - CALL_PROMPT_OFFSET,
        };
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }
}

/// The call prompt showing, if any.
#[tauri::command]
#[specta::specta]
pub fn get_call_prompt() -> Option<CallPromptLabels> {
    call_prompt().clone()
}

/// Moves the call prompt showing to the overlay's new side.
fn update_call_prompt_position(app_handle: &AppHandle) {
    let position = settings::get_settings(app_handle).overlay_position;
    let labels = {
        let mut prompt = call_prompt();
        let Some(labels) = prompt.as_mut() else {
            return;
        };
        labels.position = position;
        labels.clone()
    };
    if let Some(window) = app_handle.get_webview_window(CALL_PROMPT_WINDOW) {
        let (width, height) = current_overlay_logical_size(&window)
            .unwrap_or((COPY_OVERLAY_WIDTH, COPY_OVERLAY_HEIGHT));
        place_call_prompt(app_handle, &window, width, height);
        let _ = app_handle.emit_to(CALL_PROMPT_WINDOW, "show-overlay-call", labels);
    }
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
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
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

/// The copy notice's window around a measured capsule: CARD_MARGIN on each
/// side, never smaller than the meeting pill's window, and never taller or wider
/// than the work area left below the top offset, so the capsule stays on
/// screen whichever edge it sits at.
fn copy_window_size(capsule: (f64, f64), work_area: (f64, f64)) -> (f64, f64) {
    let fit = |measured: f64, least: f64, most: f64| {
        let wanted = if measured.is_finite() {
            measured.ceil() + 2.0 * CARD_MARGIN
        } else {
            least
        };
        wanted.min(most).max(least)
    };
    (
        fit(capsule.0, MEETING_OVERLAY_WIDTH, work_area.0),
        fit(
            capsule.1,
            MEETING_OVERLAY_HEIGHT,
            work_area.1 - OVERLAY_TOP_OFFSET,
        ),
    )
}

/// Hands the copy notice's text to its writer, outside the notice lock.
pub(crate) fn copy_held_notice() -> Result<(), String> {
    let (text, write) = match &notice().current {
        Some(Notice::Copy { text, write, .. }) => (text.clone(), Arc::clone(write)),
        _ => return Err("The copy notice is no longer showing".to_string()),
    };
    write(&text)
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

/// Whether the copy pill a Copy click came from still shows, so "Copied"
/// may take its place.
fn copy_pill_showing(notice: &Option<Notice>) -> bool {
    matches!(notice, Some(Notice::Copy { .. }))
}

/// With `hover_holds`, the notice outlasts `delay` while the pointer is over
/// it, so it does not vanish under a click on its way.
fn dismiss_notice_after(
    app_handle: &AppHandle,
    generation: u64,
    delay: Duration,
    hover_holds: bool,
) {
    let handle = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        while OVERLAY_GENERATION.load(Ordering::SeqCst) == generation {
            if !(hover_holds && pointer_over_overlay(&handle)) {
                dismiss_notice(&handle, Dismissal::Timer(generation));
                return;
            }
            std::thread::sleep(COPY_HOVER_RECHECK);
        }
    });
}

/// What ends a notice: its own timer, started at a generation, or Escape on
/// a fitted pill.
#[derive(Clone, Copy)]
enum Dismissal {
    Timer(u64),
    Escape,
    Meeting,
}

/// What the overlay shows once a notice is dismissed.
#[derive(Debug, PartialEq, Eq)]
enum AfterDismissal {
    /// The microphone warning the copy pill held back.
    Warning,
    /// The meeting pill, or nothing.
    Settled,
}

/// Replaces `notice` with the one that follows it, when `dismissal` still
/// applies at `generation`, read under the notice lock: a timer the overlay
/// moved past, such as the copy pill's after Escape showed the warning that
/// follows it, or Escape with no fitted pill showing, leaves it alone and
/// gives `None`. With the overlay turned off since the copy pill showed,
/// nothing follows it.
fn dismiss(
    notice: &mut Option<Notice>,
    dismissal: Dismissal,
    generation: u64,
    style: OverlayStyle,
) -> Option<AfterDismissal> {
    let applies = match dismissal {
        Dismissal::Timer(started) => started == generation,
        Dismissal::Escape => notice.as_ref().is_some_and(Notice::is_fitted),
        Dismissal::Meeting => matches!(notice, Some(Notice::MeetingNews(_))),
    };
    if !applies {
        return None;
    }
    *notice = notice_after(notice.take()).filter(|_| style != OverlayStyle::None);
    Some(if matches!(notice, Some(Notice::MicLost)) {
        AfterDismissal::Warning
    } else {
        AfterDismissal::Settled
    })
}

fn dismiss_notice(app_handle: &AppHandle, dismissal: Dismissal) {
    let style = settings::get_settings(app_handle).overlay_style;
    let mut held = notice();
    match dismiss(
        &mut held.current,
        dismissal,
        OVERLAY_GENERATION.load(Ordering::SeqCst),
        style,
    ) {
        None => {}
        Some(AfterDismissal::Warning) => present_mic_lost_notice(app_handle, held),
        Some(AfterDismissal::Settled) => {
            DICTATION_SHOWING.store(false, Ordering::SeqCst);
            OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
            drop(held);
            settle_overlay(app_handle);
        }
    }
}

/// The notice that takes the place of `gone`: the microphone warning a copy
/// pill held back, or none.
fn notice_after(gone: Option<Notice>) -> Option<Notice> {
    match gone {
        Some(Notice::Copy {
            mic_lost_next: true,
            ..
        }) => Some(Notice::MicLost),
        _ => None,
    }
}

/// While a fitted pill shows, the overlay takes clicks and Escape; at any
/// other time the mouse goes through it to the app below. Main thread only.
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

fn escape_dismissal(key_code: u16) -> Option<Dismissal> {
    (key_code == ESCAPE_KEY_CODE).then_some(Dismissal::Escape)
}

fn dismiss_escape_key(app_handle: &AppHandle, key_code: u16) {
    if let Some(dismissal) = escape_dismissal(key_code) {
        // Dismissing removes the monitors, so it runs outside their handlers.
        let app = app_handle.clone();
        std::thread::spawn(move || dismiss_notice(&app, dismissal));
    }
}

/// Global monitors exclude Murmur's own events. The local monitor leaves those
/// events unchanged so Escape still reaches the focused Settings control.
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

fn update_overlay_position_on_main(app_handle: &AppHandle) {
    update_call_prompt_position(app_handle);
    if settings::get_settings(app_handle).overlay_style == OverlayStyle::None
        && !DICTATION_SHOWING.load(Ordering::SeqCst)
        && !notice_showing()
    {
        settle_overlay(app_handle);
        return;
    }
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        let (width, height) = current_overlay_logical_size(&overlay_window)
            .unwrap_or((ORB_OVERLAY_SIZE, ORB_OVERLAY_SIZE));
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    }
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
mod tests {
    use super::*;

    #[test]
    fn monitor_hit_test_uses_half_open_physical_bounds() {
        let position = PhysicalPosition::new(-2560, -200);
        let size = PhysicalSize::new(2560, 1440);

        assert!(is_point_within((-2560, -200), &position, &size));
        assert!(is_point_within((-1, 1239), &position, &size));
        assert!(!is_point_within((0, 0), &position, &size));
        assert!(!is_point_within((-1, 1240), &position, &size));
    }

    #[test]
    fn with_the_overlay_off_a_prompt_over_a_meeting_leaves_nothing_on_screen() {
        let pill = MeetingPill {
            started_at_ms: 1,
            stops_at_ms: 2,
        };
        assert_eq!(resting_pill(Some(pill), OverlayStyle::None), None);
        assert_eq!(resting_pill(Some(pill), OverlayStyle::Minimal), Some(pill));
        assert_eq!(resting_pill(None, OverlayStyle::Minimal), None);
    }

    #[test]
    fn copy_window_wraps_the_measured_capsule_in_its_margins() {
        let screen = (1440.0, 875.0);
        assert_eq!(copy_window_size((300.4, 62.0), screen), (333.0, 94.0));
        assert_eq!(copy_window_size((340.0, 34.0), screen), (372.0, 66.0));
    }

    #[test]
    fn copy_window_stays_between_the_meeting_window_and_the_screen() {
        let screen = (1440.0, 875.0);
        let least = (MEETING_OVERLAY_WIDTH, MEETING_OVERLAY_HEIGHT);
        assert_eq!(copy_window_size((40.0, 12.0), screen), least);
        assert_eq!(copy_window_size((-5.0, 0.0), screen), least);
        assert_eq!(copy_window_size((f64::NAN, f64::INFINITY), screen), least);
        assert_eq!(
            copy_window_size((4000.0, 2000.0), screen),
            (1440.0, 875.0 - OVERLAY_TOP_OFFSET)
        );
    }

    /// A copy pill of the test's own, not the shared notice, whose Copy
    /// writes to the returned pasteboard.
    fn copy_pill(text: &str) -> (Option<Notice>, Arc<Mutex<Vec<String>>>) {
        let pasteboard = Arc::new(Mutex::new(Vec::new()));
        let written = Arc::clone(&pasteboard);
        let notice = Some(Notice::Copy {
            text: text.to_string(),
            write: Arc::new(move |text| {
                written.lock().unwrap().push(text.to_string());
                Ok(())
            }),
            mic_lost_next: false,
        });
        (notice, pasteboard)
    }

    /// Clicks Copy as `copy_held_notice` does.
    fn click_copy(notice: &Option<Notice>) -> Result<(), String> {
        match notice {
            Some(Notice::Copy { text, write, .. }) => write(text),
            _ => Err("The copy notice is no longer showing".to_string()),
        }
    }

    #[test]
    fn a_late_transcribing_state_or_warning_leaves_the_copy_pills_text() {
        let (mut notice, pasteboard) = copy_pill("the dictated text");

        assert!(!dictation_takes_overlay(&mut notice, "transcribing"));
        assert!(!hold_mic_lost_notice(&mut notice, &operation(1)));

        assert_eq!(click_copy(&notice), Ok(()));
        assert_eq!(pasteboard.lock().unwrap().as_slice(), ["the dictated text"]);
        assert!(
            dictation_takes_overlay(&mut notice, "recording"),
            "a new recording replaces it"
        );
        assert!(notice.is_none());
    }

    #[test]
    fn a_copy_pill_and_the_microphone_warning_show_one_after_the_other() {
        let (mut both, _) = copy_pill("the dictated text");
        if let Some(Notice::Copy { mic_lost_next, .. }) = both.as_mut() {
            *mic_lost_next = true;
        }
        assert_eq!(
            dismiss(&mut both, Dismissal::Timer(1), 1, OverlayStyle::Minimal),
            Some(AfterDismissal::Warning),
            "the warning shows once the copy pill goes"
        );
        assert_eq!(
            dismiss(&mut both, Dismissal::Timer(2), 2, OverlayStyle::Minimal),
            Some(AfterDismissal::Settled),
            "then nothing"
        );

        for mut alone in [
            copy_pill("text").0,
            Some(Notice::Error),
            Some(Notice::Cancelled),
        ] {
            assert_eq!(
                dismiss(&mut alone, Dismissal::Timer(1), 1, OverlayStyle::Minimal),
                Some(AfterDismissal::Settled)
            );
            assert!(alone.is_none());
        }
    }

    #[test]
    fn a_late_timer_leaves_the_warning_that_followed_the_copy_pill() {
        let (mut notice, _) = copy_pill("the dictated text");
        if let Some(Notice::Copy { mic_lost_next, .. }) = notice.as_mut() {
            *mic_lost_next = true;
        }
        // Escape on the copy pill shows the warning, at generation 2.
        assert_eq!(
            dismiss(&mut notice, Dismissal::Escape, 1, OverlayStyle::Minimal),
            Some(AfterDismissal::Warning)
        );
        assert!(matches!(notice, Some(Notice::MicLost)));

        // The copy pill's timer, started at generation 1, wakes late.
        assert_eq!(
            dismiss(&mut notice, Dismissal::Timer(1), 2, OverlayStyle::Minimal),
            None
        );
        assert!(
            matches!(notice, Some(Notice::MicLost)),
            "the warning keeps its time"
        );
        assert_eq!(
            dismiss(&mut notice, Dismissal::Timer(2), 2, OverlayStyle::Minimal),
            Some(AfterDismissal::Settled)
        );
        assert!(notice.is_none());

        let mut error = Some(Notice::Error);
        assert_eq!(
            dismiss(&mut error, Dismissal::Escape, 3, OverlayStyle::Minimal),
            None,
            "Escape leaves an error"
        );
        assert!(matches!(error, Some(Notice::Error)));
    }

    #[test]
    fn escape_monitors_only_dismiss_fitted_notices_for_the_escape_key() {
        for key in [0, 36, 51, ESCAPE_KEY_CODE] {
            let (mut notice, _) = copy_pill("text");
            let after = escape_dismissal(key).and_then(|dismissal| {
                dismiss(&mut notice, dismissal, 1, OverlayStyle::Minimal)
            });
            if key == ESCAPE_KEY_CODE {
                assert_eq!(after, Some(AfterDismissal::Settled));
                assert!(notice.is_none());
            } else {
                assert_eq!(after, None);
                assert!(copy_pill_showing(&notice));
            }
        }
        let mut notice = Some(Notice::Error);
        assert_eq!(
            escape_dismissal(ESCAPE_KEY_CODE).and_then(|dismissal| {
                dismiss(&mut notice, dismissal, 1, OverlayStyle::Minimal)
            }),
            None
        );
        assert!(matches!(notice, Some(Notice::Error)));
    }

    /// A copy pill whose warning follows it.
    fn copy_pill_then_warning() -> Option<Notice> {
        let (mut notice, _) = copy_pill("the dictated text");
        if let Some(Notice::Copy { mic_lost_next, .. }) = notice.as_mut() {
            *mic_lost_next = true;
        }
        notice
    }

    #[test]
    fn a_copy_click_after_the_timer_showed_the_warning_leaves_the_warning_its_time() {
        assert!(copy_pill_showing(&copy_pill("text").0));
        let mut notice = copy_pill_then_warning();
        dismiss(&mut notice, Dismissal::Timer(1), 1, OverlayStyle::Minimal);
        assert!(
            !copy_pill_showing(&notice),
            "the click arms no Copied timer over the warning"
        );
    }

    #[test]
    fn with_the_overlay_turned_off_nothing_follows_the_copy_pill() {
        let mut notice = copy_pill_then_warning();
        assert_eq!(
            dismiss(&mut notice, Dismissal::Timer(1), 1, OverlayStyle::None),
            Some(AfterDismissal::Settled)
        );
        assert!(notice.is_none());
    }

    #[test]
    fn the_microphone_warning_is_fitted_and_dismissed_like_the_copy_pill() {
        let mut notice = Some(Notice::Error);
        assert!(
            hold_mic_lost_notice(&mut notice, &operation(1)),
            "it replaces an error"
        );
        assert!(matches!(notice, Some(Notice::MicLost)));

        assert!(notice.as_ref().is_some_and(Notice::is_fitted));
        assert!(copy_pill("text").0.as_ref().is_some_and(Notice::is_fitted));
        assert!(!Notice::Error.is_fitted(), "an error pill has a fixed size");

        assert!(!dictation_takes_overlay(&mut notice, "transcribing"));
        assert!(matches!(notice, Some(Notice::MicLost)));
        assert!(dictation_takes_overlay(&mut notice, "recording"));
        assert!(notice.is_none());

        let mut error = Some(Notice::Error);
        assert!(
            dictation_takes_overlay(&mut error, "transcribing"),
            "a dictation still replaces an error"
        );
        assert!(error.is_none());
    }

    #[test]
    fn restore_replaces_the_cancelled_pill_it_was_clicked_from() {
        let mut notice = Some(Notice::Cancelled);
        assert!(notice.as_ref().is_some_and(Notice::is_fitted));
        assert!(dictation_takes_overlay(&mut notice, "transcribing"));
        assert!(notice.is_none());
    }

    #[test]
    fn a_cancelled_dictation_keeps_its_pill_but_never_covers_a_newer_dictation() {
        let shown = DictationEpoch(4);

        // Cancelled while it transcribes: it still owns the overlay when its
        // pipeline unwinds and raises the pill, before the coordinator hides it.
        let mut notice = None;
        assert!(hold_cancelled_notice(&mut notice, shown, DictationEpoch(4)));
        assert!(matches!(notice, Some(Notice::Cancelled)));

        // A newer dictation showed first: its recorder may not be open yet, or
        // it already transcribes. The old cancel's pill is dropped.
        let mut notice = None;
        assert!(dictation_takes_overlay(&mut notice, "recording"));
        assert!(!hold_cancelled_notice(
            &mut notice,
            shown,
            DictationEpoch(5)
        ));
        assert!(notice.is_none());

        // The cancel's pill came first: the new dictation replaces it, and the
        // pill's 5 s timer finds a newer overlay generation.
        let mut notice = None;
        assert!(hold_cancelled_notice(&mut notice, shown, DictationEpoch(4)));
        assert!(dictation_takes_overlay(&mut notice, "recording"));
        assert!(notice.is_none());
    }

    fn operation(id: u64) -> ProcessingOperation {
        ProcessingOperation::new(crate::OperationId(id))
    }

    #[test]
    fn a_dictation_cancelled_during_its_history_save_leaves_the_next_ones_overlay() {
        let overlay = Arc::new(Mutex::new(None::<Notice>));
        let old = operation(1);
        let (saving_tx, saving_rx) = std::sync::mpsc::channel();
        let (saved_tx, saved_rx) = std::sync::mpsc::channel::<()>();
        let pipeline = {
            let overlay = Arc::clone(&overlay);
            let old = old.clone();
            std::thread::spawn(move || {
                // Its text came back empty with the microphone lost: it
                // passed the cancel check and now saves its history row.
                assert!(!old.is_cancelled());
                saving_tx.send(()).unwrap();
                saved_rx.recv().unwrap();
                hold_mic_lost_notice(&mut overlay.lock().unwrap(), &old)
            })
        };

        saving_rx.recv().unwrap();
        // The user cancels; nothing was pasted, so the coordinator is idle
        // at once and the next dictation starts.
        assert!(old.cancel());
        assert!(dictation_takes_overlay(
            &mut overlay.lock().unwrap(),
            "recording"
        ));
        saved_tx.send(()).unwrap();

        assert!(!pipeline.join().unwrap(), "the old warning is not shown");
        let mut overlay = overlay.lock().unwrap();
        assert!(overlay.is_none());
        assert!(
            dictation_takes_overlay(&mut overlay, "transcribing"),
            "the new dictation still shows its transcribing state"
        );
    }

    #[test]
    fn a_cancel_after_the_paste_keeps_the_microphone_warning() {
        // Text already pasted cannot be taken back, so its warning stands.
        let pasted = operation(1);
        assert!(pasted.try_enter_paste().is_some());
        assert!(!pasted.cancel());
        let mut notice = None;
        assert!(hold_mic_lost_notice(&mut notice, &pasted));
        assert!(matches!(notice, Some(Notice::MicLost)));
    }
}
