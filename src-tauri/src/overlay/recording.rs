use super::*;

/// Creates the recording overlay window and keeps it hidden by default. It
/// never takes focus, so the dictation still lands in the app the user was
/// in, and clicks go through it.
#[cfg(target_os = "windows")]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    meeting_notice::listen_for_page(app_handle);
    if app_handle.get_webview_window("recording_overlay").is_some() {
        return;
    }
    meeting_notice::reset_page(app_handle);
    match tauri::WebviewWindowBuilder::new(
        app_handle,
        "recording_overlay",
        WebviewUrl::App("src/overlay/index.html".into()),
    )
    .title("Recording")
    .inner_size(MEETING_OVERLAY_WIDTH, MEETING_OVERLAY_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focusable(false)
    .focused(false)
    .visible(false)
    // A native load hook also runs when the old page cannot send unload IPC.
    .on_page_load(|window, payload| {
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
            meeting_notice::reset_page(window.app_handle());
        }
    })
    .build()
    {
        Ok(window) => {
            let _ = window.set_ignore_cursor_events(true);
            let app = app_handle.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    meeting_notice::reset_page(&app);
                }
            });
        }
        Err(e) => {
            log::error!("Failed to create recording overlay window: {}", e);
        }
    }
}

/// Creates the recording overlay panel and keeps it hidden by default.
#[cfg(target_os = "macos")]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    meeting_notice::listen_for_page(app_handle);
    if app_handle.get_webview_window("recording_overlay").is_some() {
        return;
    }
    meeting_notice::reset_page(app_handle);
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
                // A native load hook also runs when the old page cannot send unload IPC.
                .on_page_load(|window, payload| {
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                        meeting_notice::reset_page(window.app_handle());
                    }
                })
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
                let app = app_handle.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        meeting_notice::reset_page(&app);
                    }
                });
            }
        }
        Err(e) => {
            log::error!("Failed to create recording overlay panel: {}", e);
        }
    }
}
