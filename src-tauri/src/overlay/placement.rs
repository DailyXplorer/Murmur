use super::*;

#[cfg(target_os = "windows")]
pub(super) fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    crate::window_placement::monitor_under_cursor(app_handle)
}

#[cfg(target_os = "macos")]
pub(super) fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
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

pub(super) fn is_point_within(
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

/// The overlay never takes the focus, so the pointer is read from the system;
/// the cursor and the window are both in physical pixels.
#[cfg(target_os = "windows")]
pub(super) fn pointer_over_overlay(app_handle: &AppHandle) -> bool {
    let Some(window) = app_handle.get_webview_window("recording_overlay") else {
        return false;
    };
    match (
        app_handle.cursor_position(),
        window.outer_position(),
        window.outer_size(),
    ) {
        (Ok(pointer), Ok(origin), Ok(size)) => {
            is_point_within((pointer.x as i32, pointer.y as i32), &origin, &size)
        }
        _ => false,
    }
}

/// The panel never becomes key, so its webview gets no hover events; the
/// pointer is read from the window server instead. Compared in points: tao
/// scales the cursor by the primary display and the window by its own.
#[cfg(target_os = "macos")]
pub(super) fn pointer_over_overlay(app_handle: &AppHandle) -> bool {
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
#[cfg(target_os = "macos")]
pub(super) fn calculate_overlay_position(
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

/// Sizes and moves an overlay window in one native call, in the destination
/// monitor's physical pixels: tao converts logical values with the scale of
/// the monitor the window is leaving. `extra_gap` moves it further from the
/// screen edge. Also keeps it above other topmost windows, which Windows may
/// have raised over it since the last show.
#[cfg(target_os = "windows")]
pub(super) fn place_overlay(
    app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    width: f64,
    height: f64,
    extra_gap: f64,
) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE};

    let Some(monitor) = crate::window_placement::monitor_under_cursor(app_handle) else {
        return;
    };
    let bounds = crate::window_placement::overlay_bounds(
        monitor.work_area(),
        monitor.scale_factor(),
        (width, height),
        OVERLAY_BOTTOM_OFFSET - CARD_MARGIN + extra_gap,
        settings::get_settings(app_handle).overlay_position,
    );
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            log::error!("Failed to get the overlay window handle: {e}");
            return;
        }
    };
    // SAFETY: `hwnd` belongs to the live overlay window, on its UI thread.
    let placed = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            bounds.position.x,
            bounds.position.y,
            bounds.size.width as i32,
            bounds.size.height as i32,
            SWP_NOACTIVATE,
        )
    };
    if let Err(e) = placed {
        log::error!("Failed to place the recording overlay: {e}");
    }
}

pub(super) fn current_overlay_logical_size(
    window: &tauri::webview::WebviewWindow,
) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

pub(super) fn update_overlay_position_on_main(app_handle: &AppHandle) {
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
        #[cfg(target_os = "windows")]
        place_overlay(app_handle, &overlay_window, width, height, 0.0);
        #[cfg(target_os = "macos")]
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    }
}
