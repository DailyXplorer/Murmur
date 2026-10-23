use crate::input;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
use crate::tray_i18n::get_tray_translations;
use block2::RcBlock;
use objc2_app_kit::NSEventMask;
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

use tauri::WebviewUrl;

use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

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
// ignores the mouse except while the copy notice shows, so that margin never
// blocks clicks.
const CARD_MARGIN: f64 = 16.0;
// Capsule: 30 high, 108 wide at rest, up to 126 at full voice level. The width
// stays even so the centred capsule's edges sit on whole points.
const OVERLAY_WIDTH: f64 = 126.0 + 2.0 * CARD_MARGIN;
const OVERLAY_HEIGHT: f64 = 30.0 + 2.0 * CARD_MARGIN;
// Error capsule: at most 280 wide and four lines of text, so the longest
// message fits in every locale whatever the fallback font.
const ERROR_OVERLAY_WIDTH: f64 = 280.0 + 2.0 * CARD_MARGIN;
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
    /// only if the user clicks Copy.
    Copy {
        text: String,
        write: ClipboardWriter,
    },
}

/// Puts a copy notice's text on the clipboard.
pub(crate) type ClipboardWriter = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
static NOTICE: Mutex<Option<Notice>> = Mutex::new(None);

fn notice() -> MutexGuard<'static, Option<Notice>> {
    NOTICE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn notice_showing() -> bool {
    notice().is_some()
}

// Paths are spelled out because `tauri_panel!` imports these names itself.
type EscapeMonitor = objc2::rc::Retained<objc2::runtime::AnyObject>;

thread_local! {
    /// The copy notice's Escape monitor, on the main thread.
    static ESCAPE_MONITOR: RefCell<Option<EscapeMonitor>> = const { RefCell::new(None) };
}

/// Payload of `show-overlay-copy`.
#[derive(Clone, serde::Serialize)]
struct CopyNoticeLabels {
    copy: String,
    copied: String,
    /// Says there was no text field to paste into.
    note: String,
}

/// The call prompt's own panel, beside the overlay.
const CALL_PROMPT_WINDOW: &str = "call_prompt";
/// The call prompt sits this much further from the screen edge than the
/// overlay, clear of its tallest capsule, a four-line error: a dictation
/// shows beside it.
const CALL_PROMPT_OFFSET: f64 = 84.0 + 8.0;

/// Payload of `show-overlay-call`, and what `get_call_prompt` answers.
#[derive(Clone, serde::Serialize, specta::Type)]
pub struct CallPromptLabels {
    /// The call app's name.
    app: String,
    question: String,
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
    match PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, "recording_overlay")
        .url(WebviewUrl::App("src/overlay/index.html".into()))
        .title("Recording")
        .level(PanelLevel::Status)
        .size(tauri::Size::Logical(tauri::LogicalSize {
            width: OVERLAY_WIDTH,
            height: OVERLAY_HEIGHT,
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

    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
    *notice() = None;
    DICTATION_SHOWING.store(true, Ordering::SeqCst);
    let handle = app_handle.clone();
    let state = state.to_string();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) =
            present_overlay_window(&handle, &state, OVERLAY_WIDTH, OVERLAY_HEIGHT, false)
        {
            let _ = window.emit("show-overlay", state);
        }
    });
}

/// Shows a short error message in the overlay for a few seconds. Returns
/// false when the overlay is disabled, so the caller can fall back to a
/// notification.
pub fn show_error_overlay(app_handle: &AppHandle, message: String) -> bool {
    if settings::get_settings(app_handle).overlay_style == OverlayStyle::None {
        return false;
    }
    *notice() = Some(Notice::Error);
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
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
pub(crate) fn hold_copy_notice(text: String, write: ClipboardWriter) {
    *notice() = Some(Notice::Copy { text, write });
}

/// Shows the copy pill, a short note and a "Copy" action, because no text
/// field had the focus. A click hands the text to `write`, Escape dismisses
/// it. Returns false when the overlay is disabled, so there is no pill to
/// click.
pub fn show_copy_notice(app_handle: &AppHandle, text: String, write: ClipboardWriter) -> bool {
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
    hold_copy_notice(text, write);
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
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

/// Creates the call prompt's panel, hidden. It takes clicks whenever it
/// shows.
pub fn create_call_prompt(app_handle: &AppHandle) {
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

/// Asks whether to record the call `app` started, with "Record" and "Not
/// now", in a panel of its own: a dictation, an error or the meeting pill
/// never hides it. It shows even with the overlay off: the call prompt has
/// a setting of its own. The panel never takes the focus from the call.
pub fn show_call_prompt(app_handle: &AppHandle, app: &str) {
    let settings = settings::get_settings(app_handle);
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CallPromptLabels {
        app: app.to_string(),
        question: strings.call_record_question,
        accept: strings.call_record,
        decline: strings.call_not_now,
        position: settings.overlay_position,
    };
    *call_prompt() = Some(labels.clone());
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
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
        if let Some(window) = handle.get_webview_window(CALL_PROMPT_WINDOW) {
            let _ = window.hide();
            let _ = handle.emit_to(CALL_PROMPT_WINDOW, "hide-overlay-call", ());
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

/// Fits the copy notice's window to the capsule the webview measured, so a
/// note that wraps further in a wider fallback font still shows whole, and
/// the window, which takes clicks while the notice shows, stays close to it.
#[tauri::command]
#[specta::specta]
pub fn fit_copy_notice(app: AppHandle, width: f64, height: f64) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !matches!(*notice(), Some(Notice::Copy { .. })) {
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
/// side, never smaller than the dictation window, and never taller or wider
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
        fit(capsule.0, OVERLAY_WIDTH, work_area.0),
        fit(capsule.1, OVERLAY_HEIGHT, work_area.1 - OVERLAY_TOP_OFFSET),
    )
}

/// Hands the copy notice's text to its writer, outside the notice lock.
pub(crate) fn copy_held_notice() -> Result<(), String> {
    let (text, write) = match &*notice() {
        Some(Notice::Copy { text, write }) => (text.clone(), Arc::clone(write)),
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
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    dismiss_notice_after(&app, generation, COPIED_DISPLAY_TIME, false);
    Ok(())
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
                dismiss_notice(&handle);
                return;
            }
            std::thread::sleep(COPY_HOVER_RECHECK);
        }
    });
}

fn dismiss_notice(app_handle: &AppHandle) {
    *notice() = None;
    hide_recording_overlay(app_handle);
}

fn dismiss_copy_notice(app_handle: &AppHandle) {
    if matches!(*notice(), Some(Notice::Copy { .. })) {
        dismiss_notice(app_handle);
    }
}

/// While the copy notice shows, the overlay takes clicks and Escape; at any
/// other time the mouse goes through it to the app below. Main thread only.
fn set_interactive(
    app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    interactive: bool,
) {
    let _ = window.set_ignore_cursor_events(!interactive);
    ESCAPE_MONITOR.with(|slot| {
        let mut monitor = slot.borrow_mut();
        if interactive && monitor.is_none() {
            *monitor = add_escape_monitor(app_handle);
        } else if !interactive {
            if let Some(monitor) = monitor.take() {
                unsafe { objc2_app_kit::NSEvent::removeMonitor(&monitor) };
            }
        }
    });
}

/// The panel never becomes key, so Escape is watched globally, passively:
/// the key still reaches the app that has the focus.
fn add_escape_monitor(app_handle: &AppHandle) -> Option<EscapeMonitor> {
    let app = app_handle.clone();
    let handler = RcBlock::new(move |event: NonNull<objc2_app_kit::NSEvent>| {
        if unsafe { event.as_ref() }.keyCode() == ESCAPE_KEY_CODE {
            // Dismissing removes this monitor, so it runs outside the handler.
            let app = app.clone();
            std::thread::spawn(move || dismiss_copy_notice(&app));
        }
    });
    objc2_app_kit::NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::KeyDown,
        &handler,
    )
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
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        let (width, height) = current_overlay_logical_size(&overlay_window)
            .unwrap_or((OVERLAY_WIDTH, OVERLAY_HEIGHT));
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    }
}

/// Hides the recording overlay window with fade-out animation, or brings the
/// meeting pill back while a meeting records.
pub fn hide_recording_overlay(app_handle: &AppHandle) {
    if notice_showing() {
        return;
    }
    DICTATION_SHOWING.store(false, Ordering::SeqCst);
    settle_overlay(app_handle);
}

/// Leaves the overlay as it is when no dictation or notice holds it: the
/// meeting pill, or hidden, so a prompt that held it never stays behind.
fn settle_overlay(app_handle: &AppHandle) {
    let style = settings::get_settings(app_handle).overlay_style;
    match resting_pill(meeting_pill(), style) {
        Some(pill) => present_meeting_pill(app_handle, pill),
        None => fade_out_overlay(app_handle),
    }
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
        fade_out_overlay(app_handle);
    }
}

fn present_meeting_pill(app_handle: &AppHandle, pill: MeetingPill) {
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        if let Some(window) =
            present_overlay_window(&handle, "meeting", OVERLAY_WIDTH, OVERLAY_HEIGHT, false)
        {
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
        assert_eq!(copy_window_size((340.0, 30.0), screen), (372.0, 62.0));
    }

    #[test]
    fn copy_window_stays_between_the_dictation_window_and_the_screen() {
        let screen = (1440.0, 875.0);
        let least = (OVERLAY_WIDTH, OVERLAY_HEIGHT);
        assert_eq!(copy_window_size((40.0, 12.0), screen), least);
        assert_eq!(copy_window_size((-5.0, 0.0), screen), least);
        assert_eq!(copy_window_size((f64::NAN, f64::INFINITY), screen), least);
        assert_eq!(
            copy_window_size((4000.0, 2000.0), screen),
            (1440.0, 875.0 - OVERLAY_TOP_OFFSET)
        );
    }
}
