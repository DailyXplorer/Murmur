use crate::input;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
use crate::tray_i18n::get_tray_translations;
use block2::RcBlock;
use objc2_app_kit::NSEventMask;
use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
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
// Copy capsule: 30 high, growing from 108 to at most 324 wide to fit its note
// and Copy action on one line. The widest locale (ru, "Copied") needs 316.
const COPY_OVERLAY_WIDTH: f64 = 324.0 + 2.0 * CARD_MARGIN;
const COPY_OVERLAY_HEIGHT: f64 = OVERLAY_HEIGHT;
const COPY_DISPLAY_TIME: Duration = Duration::from_secs(4);
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
    /// No text field had the focus; the text is on the clipboard.
    Copy {
        text: String,
    },
}
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
    /// Says nothing was pasted and the text is on the clipboard.
    note: String,
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
                if is_mouse_within_monitor(mouse_location, &pos, &size) {
                    return Some(monitor);
                }
            }
        }
    }

    app_handle.primary_monitor().ok().flatten()
}

fn is_mouse_within_monitor(
    mouse_pos: (i32, i32),
    monitor_pos: &PhysicalPosition<i32>,
    monitor_size: &PhysicalSize<u32>,
) -> bool {
    let (mouse_x, mouse_y) = mouse_pos;
    let PhysicalPosition {
        x: monitor_x,
        y: monitor_y,
    } = *monitor_pos;
    let PhysicalSize {
        width: monitor_width,
        height: monitor_height,
    } = *monitor_size;

    mouse_x >= monitor_x
        && mouse_x < (monitor_x + monitor_width as i32)
        && mouse_y >= monitor_y
        && mouse_y < (monitor_y + monitor_height as i32)
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
    dismiss_notice_after(app_handle, generation, ERROR_DISPLAY_TIME);
    true
}

/// Shows the copy pill, a short note and a "Copy" action, after the text went
/// on the clipboard because no text field had the focus. A click copies the
/// text again, Escape dismisses it.
pub fn show_copy_notice(app_handle: &AppHandle, text: String) {
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }
    let strings = get_tray_translations(Some(settings.app_language));
    let labels = CopyNoticeLabels {
        copy: strings.overlay_copy,
        copied: strings.overlay_copied,
        note: strings.overlay_not_pasted,
    };
    *notice() = Some(Notice::Copy { text });
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
    dismiss_notice_after(app_handle, generation, COPY_DISPLAY_TIME);
}

fn copy_notice_text() -> Option<String> {
    match &*notice() {
        Some(Notice::Copy { text }) => Some(text.clone()),
        _ => None,
    }
}

/// The copy pill's click: puts its text on the clipboard again, then lets
/// the pill read "Copied" for a moment before it hides.
#[tauri::command]
#[specta::specta]
pub async fn copy_notice_again(app: AppHandle) -> Result<(), String> {
    let text = copy_notice_text().ok_or("The copy notice is no longer showing")?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(crate::clipboard::copy_to_keep(&clipboard_app, &text));
    })
    .map_err(|error| format!("Cannot reach the clipboard: {error}"))?;
    receiver
        .await
        .map_err(|_| "The clipboard write did not run".to_string())??;
    let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    dismiss_notice_after(&app, generation, COPIED_DISPLAY_TIME);
    Ok(())
}

fn dismiss_notice_after(app_handle: &AppHandle, generation: u64, delay: Duration) {
    let handle = app_handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if OVERLAY_GENERATION.load(Ordering::SeqCst) == generation {
            dismiss_notice(&handle);
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
    if let Some(pill) = meeting_pill() {
        present_meeting_pill(app_handle, pill);
        return;
    }
    fade_out_overlay(app_handle);
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
        present_meeting_pill(app_handle, pill);
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
    if settings::get_settings(app_handle).overlay_style == OverlayStyle::None {
        return;
    }
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

        assert!(is_mouse_within_monitor((-2560, -200), &position, &size));
        assert!(is_mouse_within_monitor((-1, 1239), &position, &size));
        assert!(!is_mouse_within_monitor((0, 0), &position, &size));
        assert!(!is_mouse_within_monitor((-1, 1240), &position, &size));
    }
}
