use super::*;
use crate::single_instance_actions::SingleInstanceAction;

#[cfg(target_os = "macos")]
pub(crate) fn settings_window_collection_behavior(
    mut behavior: objc2_app_kit::NSWindowCollectionBehavior,
) -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior;

    behavior.remove(NSWindowCollectionBehavior::CanJoinAllSpaces);
    behavior.insert(NSWindowCollectionBehavior::MoveToActiveSpace);
    behavior
}

#[cfg(target_os = "macos")]
pub(crate) fn settings_window_needs_order_out(is_visible: bool, is_on_active_space: bool) -> bool {
    is_visible && !is_on_active_space
}

#[cfg(target_os = "macos")]
pub(crate) fn prepare_settings_window_for_active_space(
    main_window: &tauri::WebviewWindow,
) -> tauri::Result<()> {
    use objc2_app_kit::NSWindow;

    let native_window = main_window.ns_window()?.cast::<NSWindow>();

    // Callers run this on the main thread, as AppKit requires. The pointer
    // belongs to the live WebviewWindow and is only borrowed here.
    unsafe {
        let native_window = &*native_window;
        let behavior = settings_window_collection_behavior(native_window.collectionBehavior());
        native_window.setCollectionBehavior(behavior);

        // MoveToActiveSpace only takes effect while AppKit orders a window in.
        // If the settings are already visible on another Space, show() is a
        // no-op and focusing them switches Spaces instead. Order them out first
        // so the normal show/focus path attaches them to the current Space.
        if settings_window_needs_order_out(
            native_window.isVisible(),
            native_window.isOnActiveSpace(),
        ) {
            native_window.orderOut(None);
        }
    }

    Ok(())
}

/// Logical content size the settings window opens at, before it is fitted to the screen.
pub(crate) const SETTINGS_WINDOW_SIZE: (f64, f64) = (780.0, 720.0);

/// The smallest content size every settings page fits in.
pub(crate) const SETTINGS_WINDOW_MIN_SIZE: (f64, f64) = (680.0, 570.0);

/// Index of the first screen frame holding `point`, edges included, all in
/// AppKit's global coordinates.
#[cfg(target_os = "macos")]
pub(crate) fn screen_containing(
    frames: &[objc2_foundation::NSRect],
    point: objc2_foundation::NSPoint,
) -> Option<usize> {
    frames.iter().position(|frame| {
        (frame.min().x..=frame.max().x).contains(&point.x)
            && (frame.min().y..=frame.max().y).contains(&point.y)
    })
}

/// The frame origin that centers a window of `size` in `visible`, a screen's
/// frame without the menu bar and Dock, as Rectangle's Center does.
/// NSWindow.center() would sit it "somewhat above center" instead. A window
/// too large for the area keeps its left edge and title bar inside it.
#[cfg(target_os = "macos")]
pub(crate) fn centered_window_origin(
    visible: objc2_foundation::NSRect,
    size: objc2_foundation::NSSize,
) -> objc2_foundation::NSPoint {
    let x = if size.width > visible.size.width {
        visible.origin.x
    } else {
        visible.origin.x + ((visible.size.width - size.width) / 2.0).floor()
    };
    let y = if size.height > visible.size.height {
        visible.max().y - size.height
    } else {
        visible.origin.y + ((visible.size.height - size.height) / 2.0).floor()
    };
    objc2_foundation::NSPoint::new(x, y)
}

/// The (width, height) that fits a window of `size` in `visible`, the screen
/// without the menu bar and Dock or the taskbar, never smaller than `min`.
/// All include the title bar and borders, in one unit.
pub(crate) fn fitted_window_size(
    visible: (f64, f64),
    size: (f64, f64),
    min: (f64, f64),
) -> (f64, f64) {
    (
        size.0.min(visible.0).max(min.0),
        size.1.min(visible.1).max(min.1),
    )
}

/// Centers the settings window on the screen under the pointer. A window too
/// tall or wide for that screen, such as a 13" 1280x800 display with the Dock
/// showing, shrinks to fit, never below its minimum size. Every show of the
/// window goes through here.
#[cfg(target_os = "macos")]
pub(crate) fn center_settings_window_on_pointer_screen(
    main_window: &tauri::WebviewWindow,
) -> tauri::Result<()> {
    use objc2_app_kit::{NSEvent, NSScreen, NSWindow, NSWindowStyleMask};

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        log::error!("Settings window centering must run on the main thread");
        return Ok(());
    };
    let native_window = main_window.ns_window()?.cast::<NSWindow>();

    // The NSWindow belongs to the live WebviewWindow and is only borrowed here.
    unsafe {
        let native_window = &*native_window;
        if native_window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
        {
            return Ok(());
        }
        let screens = NSScreen::screens(mtm).to_vec();
        let frames: Vec<_> = screens.iter().map(|screen| screen.frame()).collect();
        let screen = screen_containing(&frames, NSEvent::mouseLocation())
            .map(|index| screens[index].clone())
            .or_else(|| native_window.screen())
            .or_else(|| NSScreen::mainScreen(mtm));
        if let Some(screen) = screen {
            let visible = screen.visibleFrame();
            let frame = native_window.frame().size;
            // tao stores min_inner_size as the frame's minSize, title bar included.
            let min = native_window.minSize();
            let (width, height) = fitted_window_size(
                (visible.size.width, visible.size.height),
                (frame.width, frame.height),
                (min.width, min.height),
            );
            let size = objc2_foundation::NSSize::new(width, height);
            let origin = centered_window_origin(visible, size);
            native_window.setFrame_display(objc2_foundation::NSRect::new(origin, size), true);
        }
    }

    Ok(())
}

/// Centers the settings window in the work area of the monitor under the
/// pointer, as the macOS build does with its screens. A window too tall or
/// wide for that work area, such as 1024x768 less the taskbar, shrinks to
/// fit. On small screens only, its minimum yields to the available area.
#[cfg(target_os = "windows")]
pub(crate) fn center_settings_window_on_pointer_screen(
    main_window: &tauri::WebviewWindow,
) -> tauri::Result<()> {
    if main_window.is_fullscreen()? || main_window.is_maximized()? {
        return Ok(());
    }
    let Some(monitor) = window_placement::monitor_under_cursor(main_window.app_handle()) else {
        return Ok(());
    };
    // The window may still sit on a monitor with another scale; its sizes on
    // the destination are its logical sizes at the destination's scale.
    let scale = main_window.scale_factor()?;
    let (inner_size, position) = window_placement::fitted_and_centered(
        monitor.work_area(),
        monitor.scale_factor(),
        main_window.outer_size()?.to_logical(scale),
        main_window.inner_size()?.to_logical(scale),
        SETTINGS_WINDOW_MIN_SIZE,
    );
    // Update the native constraint first: otherwise Windows clamps set_size
    // back to 680x570 even when this monitor has less usable client space.
    // Returning to a larger monitor restores the usual minimum.
    main_window.set_min_size(Some(tauri::LogicalSize::new(
        inner_size.width.min(SETTINGS_WINDOW_MIN_SIZE.0),
        inner_size.height.min(SETTINGS_WINDOW_MIN_SIZE.1),
    )))?;
    main_window.set_size(inner_size)?;
    main_window.set_position(position)
}

/// Builds the settings window, hidden. Closing it can destroy it, so every
/// open after the first may build it again.
pub(crate) fn build_main_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    // A rebuilt window opens at the size the user left it at; its position is
    // chosen by `center_settings_window_on_pointer_screen` at every show.
    #[cfg(target_os = "windows")]
    let size = settings_window::remembered_size(app).unwrap_or(SETTINGS_WINDOW_SIZE);
    #[cfg(not(target_os = "windows"))]
    let size = SETTINGS_WINDOW_SIZE;
    let window = tauri::WebviewWindowBuilder::new(
        app,
        settings_window::MAIN_WINDOW,
        tauri::WebviewUrl::App("/".into()),
    )
    .title("Murmur")
    .inner_size(size.0, size.1)
    .min_inner_size(SETTINGS_WINDOW_MIN_SIZE.0, SETTINGS_WINDOW_MIN_SIZE.1)
    .resizable(true)
    .maximizable(true)
    .visible(false)
    .build()?;
    // Apply the persisted appearance theme to the native title bar before
    // the window is shown, so it matches the in-app palette without a flash
    // of the wrong theme.
    shortcut::apply_window_theme(app, get_settings(app).theme);
    // The taskbar shows the window's own icon, which a new window lacks.
    #[cfg(target_os = "windows")]
    if let Err(error) = accent::apply_native_accent(app, get_settings(app).accent_color) {
        log::warn!("Failed to apply native accent icon: {error}");
    }
    Ok(window)
}

/// The logical inner size of a settings window that is neither full screen,
/// maximized nor minimized.
#[cfg(target_os = "windows")]
pub(crate) fn settings_window_restorable_size(
    window: &tauri::Window,
) -> tauri::Result<Option<(f64, f64)>> {
    if window.is_fullscreen()? || window.is_maximized()? || window.is_minimized()? {
        return Ok(None);
    }
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    Ok(Some((size.width, size.height)))
}

/// Keeps the size of the settings window about to be destroyed.
#[cfg(target_os = "windows")]
pub(crate) fn remember_settings_window_size(window: &tauri::Window) {
    match settings_window_restorable_size(window) {
        Ok(Some(size)) => settings_window::remember_size(window.app_handle(), size),
        Ok(None) => {}
        Err(error) => log::warn!("Failed to keep the settings window size: {error}"),
    }
}

/// Brings the settings window forward, building it first when a close
/// destroyed it.
pub(crate) fn show_main_window(app: &AppHandle) {
    // WebView2 deadlocks a window built inside an event handler on Windows,
    // such as the tray menu or the second launch's callback: the build waits
    // on a thread of its own, which the event loop can serve.
    #[cfg(target_os = "windows")]
    if app
        .get_webview_window(settings_window::MAIN_WINDOW)
        .is_none()
    {
        let app = app.clone();
        std::thread::spawn(move || show_or_build_main_window(&app));
        return;
    }
    show_or_build_main_window(app);
}

pub(crate) fn show_or_build_main_window(app: &AppHandle) {
    let main_window = match app.get_webview_window(settings_window::MAIN_WINDOW) {
        Some(window) => Some(window),
        None => {
            let built = build_main_window(app);
            // Two opens in a row may build at once: the loser finds the
            // winner's.
            #[cfg(target_os = "windows")]
            let built = built.or_else(|error| {
                app.get_webview_window(settings_window::MAIN_WINDOW)
                    .ok_or(error)
            });
            built
                .inspect_err(|e| log::error!("Failed to build the settings window: {}", e))
                .ok()
        }
    };
    if let Some(main_window) = main_window {
        let app = app.clone();
        let window = main_window.clone();
        if let Err(e) = main_window.run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            if let Err(e) = prepare_settings_window_for_active_space(&window) {
                log::error!("Failed to prepare settings window for active Space: {}", e);
            }
            if let Err(e) = window.unminimize() {
                log::error!("Failed to unminimize webview window: {}", e);
            }
            if let Err(e) = center_settings_window_on_pointer_screen(&window) {
                log::error!("Failed to center settings window: {}", e);
            }
            if let Err(e) = window.show() {
                log::error!("Failed to show webview window: {}", e);
            }
            if let Err(e) = window.set_focus() {
                log::error!("Failed to focus webview window: {}", e);
            }
            #[cfg(target_os = "macos")]
            accent::set_dock_presence(&app, accent::DockPresence::Shown);
            overlay::settings_meeting_changed(&app);
        }) {
            log::error!("Failed to schedule settings window presentation: {}", e);
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

pub(crate) fn dispatch_single_instance_action(app: &AppHandle, action: SingleInstanceAction) {
    match action {
        SingleInstanceAction::Show => show_main_window(app),
        SingleInstanceAction::Toggle => {
            signal_handle::send_transcription_input(app, "transcribe", "CLI");
        }
        // Queue cancellation through the coordinator so a preceding remote
        // toggle has started its recording before cancellation inspects it.
        // The fallback keeps this dispatch safe if it is ever reused before
        // coordinator initialization; normal single-instance startup buffers
        // actions until after that initialization.
        SingleInstanceAction::Cancel => {
            if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                coordinator.send_remote_cancel();
            } else {
                crate::utils::cancel_current_operation_before_coordinator(app);
            }
        }
    }
}
