use super::*;

/// Creates the call prompt's window, hidden. It takes clicks but never the
/// focus, so the call keeps it. It only lives while a prompt shows. The build
/// waits for the event loop, so it runs on the call watcher's thread, never
/// on the main one: WebView2 deadlocks a window built there.
#[cfg(target_os = "windows")]
pub(super) fn create_call_prompt(app_handle: &AppHandle) {
    if let Err(e) = tauri::WebviewWindowBuilder::new(
        app_handle,
        CALL_PROMPT_WINDOW,
        WebviewUrl::App("src/overlay/call.html".into()),
    )
    .title("Call")
    .inner_size(COPY_OVERLAY_WIDTH, COPY_OVERLAY_HEIGHT)
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
    .build()
    {
        log::error!("Failed to create the call prompt window: {}", e);
    }
}

/// Creates the call prompt's panel, hidden. It takes clicks whenever it
/// shows. It only lives while a prompt shows.
#[cfg(target_os = "macos")]
pub(super) fn create_call_prompt(app_handle: &AppHandle) {
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
/// panel never takes the focus from the call. Call it from a thread of its
/// own, as the call watcher does: on Windows it builds the window.
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
    #[cfg(target_os = "windows")]
    if app_handle.get_webview_window(CALL_PROMPT_WINDOW).is_none() {
        create_call_prompt(app_handle);
    }
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        // A page built here asks for the labels once it listens.
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "windows")]
        let window = handle.get_webview_window(CALL_PROMPT_WINDOW);
        if let Some(window) = window {
            let _ = window.hide();
            if let Err(e) = window.destroy() {
                log::error!("Failed to destroy the call prompt panel: {}", e);
            }
        }
    });
}

#[cfg(target_os = "windows")]
pub(super) fn place_call_prompt(
    app_handle: &AppHandle,
    window: &tauri::webview::WebviewWindow,
    width: f64,
    height: f64,
) {
    place_overlay(app_handle, window, width, height, CALL_PROMPT_OFFSET);
}

#[cfg(target_os = "macos")]
pub(super) fn place_call_prompt(
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

/// Moves the call prompt showing to the overlay's new side.
pub(super) fn update_call_prompt_position(app_handle: &AppHandle) {
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
