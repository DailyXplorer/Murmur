#[cfg(not(any(target_os = "macos", target_os = "windows")))]
compile_error!("Murmur supports macOS and Windows only.");

mod accent;
#[cfg(target_os = "macos")]
mod accessibility;
mod actions;
mod audio_feedback;
pub mod audio_toolkit;
mod autostart;
mod chinese_script;
mod cleanup;
pub mod cli;
mod clipboard;
mod codex_auth;
mod codex_login;
mod codex_transcribe;
mod commands;
mod dictation_gesture;
mod dictionary;
mod engine;
// Windows reads no field (see `window_names`); only the excerpt type is used.
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod field_excerpt;
#[cfg(target_os = "windows")]
mod foreground;
mod helpers;
mod input;
mod insights;
pub mod local_models;
mod managers;
mod meetings;
mod operation;
mod overlay;
mod paste_last;
mod paste_tx;
mod settings;
mod shortcut;
mod signal_handle;
mod single_instance_actions;
mod transcription_coordinator;
mod transcription_error;
mod tray;
mod tray_i18n;
mod updates;
mod user_alert;
mod utils;
mod window_names;
#[cfg(target_os = "windows")]
mod window_placement;

pub use cli::CliArgs;
pub use operation::{OperationId, PastePermit, ProcessingOperation};
#[cfg(any(debug_assertions, test))]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::{DatabaseStartup, HistoryManager};
use managers::transcription::TranscriptionManager;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;
use crate::single_instance_actions::{
    action_from_args, SingleInstanceAction, SingleInstanceActionQueue,
};

/// Types each command's rejection instead of `any`: a `string` error as
/// `String(e)`, an error enum as its own type.
#[cfg(any(debug_assertions, test))]
fn normalize_generated_bindings_source(generated: &str) -> std::io::Result<String> {
    let pattern = regex::Regex::new(
        r"(?s)(Promise\s*<\s*Result\s*<.*?,\s*(\w+)\s*>\s*>\s*\{.*?)error:\s*e\s+as\s+any",
    )
    .expect("generated-binding error pattern is valid");
    let mut normalized_error_count = 0;
    let normalized = pattern
        .replace_all(generated, |captures: &regex::Captures| {
            normalized_error_count += 1;
            let error = match &captures[2] {
                "string" => "String(e)".to_string(),
                named => format!("e as {named}"),
            };
            format!("{}error: {error}", &captures[1])
        })
        .into_owned();

    let result_pattern = regex::Regex::new(r"Promise\s*<\s*Result\s*<")
        .expect("generated-binding Result pattern is valid");
    let result_count = result_pattern.find_iter(&normalized).count();

    if result_count != normalized_error_count {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "generated {result_count} Result bindings but normalized {normalized_error_count} string-error wrappers"
            ),
        ));
    }

    Ok(normalized)
}

pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the webview through the
/// `log://log` event for the debug panel's live log viewer. It follows debug
/// mode, because that viewer is the only consumer and only exists in debug
/// mode, so normal runs never broadcast log records (which can include file
/// paths and diagnostics) onto the frontend event bus. Synced at startup
/// and whenever debug mode is toggled (see `shortcut::change_debug_mode_setting`).
pub static WEBVIEW_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

fn level_filter_from_u8(value: u8) -> log::LevelFilter {
    match value {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        5 => log::LevelFilter::Trace,
        _ => log::LevelFilter::Trace,
    }
}

fn build_console_filter() -> env_filter::Filter {
    let mut builder = EnvFilterBuilder::new();

    match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => {
            if let Err(err) = builder.try_parse(&spec) {
                log::warn!(
                    "Ignoring invalid RUST_LOG value '{}': {}. Falling back to info-level console logging",
                    spec,
                    err
                );
                builder.filter_level(log::LevelFilter::Info);
            }
        }
        _ => {
            builder.filter_level(log::LevelFilter::Info);
        }
    }

    builder.build()
}

#[cfg(target_os = "macos")]
fn settings_window_collection_behavior(
    mut behavior: objc2_app_kit::NSWindowCollectionBehavior,
) -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior;

    behavior.remove(NSWindowCollectionBehavior::CanJoinAllSpaces);
    behavior.insert(NSWindowCollectionBehavior::MoveToActiveSpace);
    behavior
}

#[cfg(target_os = "macos")]
fn settings_window_needs_order_out(is_visible: bool, is_on_active_space: bool) -> bool {
    is_visible && !is_on_active_space
}

#[cfg(target_os = "macos")]
fn prepare_settings_window_for_active_space(
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
const SETTINGS_WINDOW_SIZE: (f64, f64) = (780.0, 720.0);
/// The smallest content size every settings page fits in.
const SETTINGS_WINDOW_MIN_SIZE: (f64, f64) = (680.0, 570.0);

/// Index of the first screen frame holding `point`, edges included, all in
/// AppKit's global coordinates.
#[cfg(target_os = "macos")]
fn screen_containing(
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
fn centered_window_origin(
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

/// The frame size that fits a window of `size` in `visible`, a screen's frame
/// without the menu bar and Dock, never smaller than `min`. All are frame sizes.
#[cfg(target_os = "macos")]
fn fitted_window_size(
    visible: objc2_foundation::NSSize,
    size: objc2_foundation::NSSize,
    min: objc2_foundation::NSSize,
) -> objc2_foundation::NSSize {
    objc2_foundation::NSSize::new(
        size.width.min(visible.width).max(min.width),
        size.height.min(visible.height).max(min.height),
    )
}

/// Centers the settings window on the screen under the pointer. A window too
/// tall or wide for that screen, such as a 13" 1280x800 display with the Dock
/// showing, shrinks to fit, never below its minimum size. Every show of the
/// window goes through here.
#[cfg(target_os = "macos")]
fn center_settings_window_on_pointer_screen(
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
            // tao stores min_inner_size as the frame's minSize, title bar included.
            let size = fitted_window_size(
                visible.size,
                native_window.frame().size,
                native_window.minSize(),
            );
            let origin = centered_window_origin(visible, size);
            native_window.setFrame_display(objc2_foundation::NSRect::new(origin, size), true);
        }
    }

    Ok(())
}

/// Centers the settings window in the work area of the monitor under the
/// pointer, as the macOS build does with its screens.
#[cfg(target_os = "windows")]
fn center_settings_window_on_pointer_screen(
    main_window: &tauri::WebviewWindow,
) -> tauri::Result<()> {
    if main_window.is_fullscreen()? || main_window.is_maximized()? {
        return Ok(());
    }
    let Some(monitor) = window_placement::monitor_under_cursor(main_window.app_handle()) else {
        return Ok(());
    };
    // The window may still sit on a monitor with another scale; its size on
    // the destination is its logical size at the destination's scale.
    let size = main_window
        .outer_size()?
        .to_logical::<f64>(main_window.scale_factor()?)
        .to_physical::<u32>(monitor.scale_factor());
    let position = window_placement::centered_in(monitor.work_area(), size);
    main_window.set_position(position)
}

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
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

fn dispatch_single_instance_action(app: &AppHandle, action: SingleInstanceAction) {
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

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    let transcription_manager = Arc::new(TranscriptionManager::new(app_handle));
    let (recording_manager, microphone_error) = AudioRecordingManager::new(app_handle);
    let recording_manager = Arc::new(recording_manager);
    if microphone_error.is_some() {
        user_alert::show(
            app_handle,
            user_alert::Alert::MicrophoneUnavailableAtStartup,
        );
    }
    // Only an unresolvable app data directory can fail here; a broken
    // database is handled inside and reported below.
    let (history_manager, history_startup) =
        HistoryManager::new(app_handle).expect("Failed to resolve the app data directory");
    let history_manager = Arc::new(history_manager);
    match history_startup {
        DatabaseStartup::Ready => {}
        DatabaseStartup::Reset { .. } => {
            user_alert::show(app_handle, user_alert::Alert::HistoryReset)
        }
        DatabaseStartup::Unavailable => {
            user_alert::show(app_handle, user_alert::Alert::HistoryUnavailable)
        }
    }

    app_handle.manage(recording_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());
    history_manager.clone().start_retention_sweep();
    // Insights are optional: a broken stats database must not stop dictation.
    match insights::InsightsManager::new(app_handle, &history_manager) {
        Ok(insights_manager) => {
            app_handle.manage(Arc::new(insights_manager));
        }
        Err(e) => log::error!("Failed to initialize insights: {e}"),
    }
    local_models::init(app_handle);
    engine::local::init(app_handle);
    meetings::init(app_handle, true);
    meetings::call_watch::sync(app_handle);
    app_handle.manage(tray::CurrentTrayIconState::new());
    updates::init(app_handle);

    // Note: Shortcuts are NOT initialized here.
    // The frontend calls `initialize_shortcuts` after permissions are confirmed.

    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone());

    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            accent::set_dock_presence(app_handle, accent::DockPresence::Hidden);
        }
    }
    let initial_icon = accent::tray_icon().expect("failed to build initial tray icon");

    let tray_builder = TrayIconBuilder::new()
        .icon(initial_icon)
        .tooltip(tray::tray_tooltip())
        .icon_as_template(accent::TRAY_ICON_IS_TEMPLATE)
        .show_menu_on_left_click(true);

    let tray = tray_builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" | updates::TRAY_ITEM_ID => {
                show_main_window(app);
            }
            "check_updates" => {
                let settings = settings::get_settings(app);
                if settings.update_checks_enabled {
                    show_main_window(app);
                    let _ = app.emit("check-for-updates", ());
                }
            }
            "copy_last_transcript" => {
                tray::copy_last_transcript(app);
            }
            "paste_last_transcript" => {
                paste_last::request(app);
            }
            meetings::TRAY_ITEM_ID => {
                meetings::request_toggle(app);
            }
            "cancel" => {
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    coordinator.send_cancel();
                } else {
                    crate::utils::cancel_current_operation_before_coordinator(app);
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    utils::update_tray_menu(app_handle, None);

    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    autostart::apply_autostart(app_handle, settings.autostart_enabled);

    utils::create_recording_overlay(app_handle);
    #[cfg(target_os = "macos")]
    if meetings::call_watch::supported() {
        utils::create_call_prompt(app_handle);
    }
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

/// Left in the app data folder by an update relaunch. The restarted process
/// gets the same arguments as before and would apply "Start hidden" again;
/// this marker tells it the user is waiting for the window instead.
const UPDATE_RELAUNCH_MARKER: &str = "show-settings-after-update";

fn leave_update_relaunch_marker(app_data_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(app_data_dir)?;
    std::fs::write(app_data_dir.join(UPDATE_RELAUNCH_MARKER), b"")
}

/// Whether this launch follows an update relaunch. The marker is removed, so
/// it shows the window once and a later login start stays hidden.
fn take_update_relaunch_marker(app_data_dir: &std::path::Path) -> bool {
    std::fs::remove_file(app_data_dir.join(UPDATE_RELAUNCH_MARKER)).is_ok()
}

/// Restarts into the installed update and shows the settings window again.
#[tauri::command]
#[specta::specta]
fn relaunch_after_update(app: AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if let Err(e) = leave_update_relaunch_marker(&app_data_dir) {
        // The update is installed either way; only the window stays hidden.
        log::warn!("Failed to mark the update relaunch: {e}");
    }
    app.request_restart();
    Ok(())
}

/// Convert an unexpected panic on the headless worker into a normal CLI
/// failure. Without this guard the Tauri event loop remains alive after the
/// worker exits, leaving `--transcribe-file` hung indefinitely.
fn run_headless_guarded<F>(operation: F) -> i32
where
    F: FnOnce() -> i32,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(code) => code,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("error: headless transcription panicked: {message}");
            1
        }
    }
}

/// Headless one-shot transcription for `--transcribe-file`.
/// Drives the same `TranscriptionManager::transcribe` path as the desktop app.
/// Returns a process exit code (0 ok, 1 runtime failure, 2 bad input/usage).
fn run_headless_transcription(app: &AppHandle, args: &CliArgs) -> i32 {
    use std::time::Instant;

    let Some(wav) = args.transcribe_file.clone() else {
        return 0;
    };

    // read_wav_samples decodes every file as 16-bit int samples without checking
    // the format. The app only saves 16 kHz mono 16-bit PCM, so reject anything
    // else instead of transcribing a mis-decoded or mis-timed signal.
    match hound::WavReader::open(&wav) {
        Ok(reader) => {
            let spec = reader.spec();
            if spec.sample_rate != 16_000
                || spec.channels != 1
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
            {
                eprintln!(
                    "error: expected 16 kHz mono 16-bit PCM WAV, got {} Hz / {} ch / {}-bit {:?}",
                    spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format
                );
                return 2;
            }
        }
        Err(e) => {
            eprintln!("error: cannot open {}: {}", wav.display(), e);
            return 2;
        }
    }

    let samples = match crate::audio_toolkit::read_wav_samples(&wav) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {}", wav.display(), e);
            return 2;
        }
    };
    let audio_secs = samples.len() as f64 / 16_000.0;
    let tm = app.state::<Arc<TranscriptionManager>>();
    let started = Instant::now();
    let text = match tm.transcribe_sync(samples) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: transcribe failed: {error}");
            return 1;
        }
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let rtf = if elapsed_ms > 0 {
        audio_secs / (elapsed_ms as f64 / 1000.0)
    } else {
        0.0
    };

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "backend": "chatgpt-session",
                "audio_secs": audio_secs,
                "transcribe_ms": elapsed_ms,
                "rtf": rtf,
                "text": text,
            })
        );
    } else {
        println!(
            "backend=chatgpt-session audio={audio_secs:.2}s transcribe={elapsed_ms}ms rtf={rtf:.2}x"
        );
        println!("text: {}", text);
    }
    0
}

/// Every command and event the webview can use.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::get_shortcut_registration_errors,
            shortcut::get_key_labels,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_theme_setting,
            shortcut::change_accent_color_setting,
            shortcut::change_start_hidden_setting,
            shortcut::change_autostart_setting,
            shortcut::change_selected_language_setting,
            shortcut::change_overlay_position_setting,
            shortcut::change_overlay_style_setting,
            shortcut::change_debug_mode_setting,
            shortcut::change_word_correction_threshold_setting,
            shortcut::change_extra_recording_buffer_setting,
            shortcut::change_paste_delay_ms_setting,
            shortcut::change_paste_delay_after_ms_setting,
            shortcut::change_reliable_paste_setting,
            shortcut::change_paste_method_setting,
            shortcut::change_clipboard_handling_setting,
            shortcut::change_auto_submit_setting,
            shortcut::change_auto_submit_key_setting,
            shortcut::change_experimental_enabled_setting,
            shortcut::update_custom_words,
            shortcut::suspend_all_bindings,
            shortcut::resume_all_bindings,
            shortcut::change_mute_while_recording_setting,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            commands::cleanup::change_ai_cleanup_setting,
            commands::cleanup::change_dictation_style_setting,
            commands::cleanup::change_window_names_setting,
            commands::cleanup::change_whisper_mode_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_show_whats_new_on_update_setting,
            shortcut::change_whats_new_last_seen_version_setting,
            shortcut::change_show_tray_icon_setting,
            show_main_window_command,
            relaunch_after_update,
            updates::get_update_status,
            updates::check_for_updates,
            commands::cancel_operation,
            overlay::copy_notice_again,
            overlay::fit_copy_notice,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::repair_accessibility_permission,
            commands::open_microphone_settings,
            commands::get_microphone_authorization_status,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::audio::update_microphone_mode,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_microphone_channels,
            commands::audio::set_selected_channel,
            commands::transcription::get_codex_auth_status,
            commands::chatgpt::sign_in_with_chatgpt,
            commands::chatgpt::cancel_chatgpt_sign_in,
            commands::chatgpt::sign_out_of_chatgpt,
            commands::transcription::complete_onboarding,
            commands::transcription::change_transcription_engine_setting,
            commands::transcription::switch_to_local_when_ready,
            commands::transcription::change_local_stt_card_dismissed_setting,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            helpers::clamshell::is_laptop,
            insights::get_insights,
            commands::meetings::get_meeting_runtime,
            commands::meetings::start_meeting,
            commands::meetings::stop_meeting,
            commands::meetings::discard_meeting,
            commands::meetings::get_meetings,
            commands::meetings::delete_meeting,
            commands::meetings::get_meeting,
            commands::meetings::rename_meeting,
            commands::meetings::rename_speaker,
            commands::meetings::retry_meeting,
            commands::meetings::reanalyze_meeting,
            commands::meetings::retry_meeting_summary,
            commands::meetings::delete_meeting_audio,
            commands::meetings::retranscribe_meeting,
            commands::local_models::get_local_model_status,
            commands::local_models::download_local_model,
            commands::local_models::cancel_local_model_download,
            commands::local_models::delete_local_model,
            commands::meetings::copy_meeting_markdown,
            commands::meetings::export_meeting_markdown,
            commands::meetings::get_meeting_capabilities,
            commands::meetings::get_system_audio_permission,
            commands::meetings::open_system_audio_privacy_settings,
            commands::meetings::change_meetings_enabled_setting,
            commands::meetings::change_meeting_capture_mode_setting,
            commands::meetings::change_meeting_audio_retention_setting,
            commands::meetings::get_meetings_storage,
            commands::meetings::change_meeting_call_prompt_setting,
            commands::meetings::answer_call_prompt,
            overlay::fit_call_prompt,
            overlay::get_call_prompt,
        ])
        .events(collect_events![
            managers::history::HistoryUpdatePayload,
            insights::InsightsUpdated,
            meetings::MeetingStateChanged,
            meetings::MeetingsUpdated,
            meetings::processor::MeetingProgress,
            local_models::LocalModelStatus,
            local_models::SwitchedToLocal,
            updates::UpdateStatus
        ])
}

/// The contents of `src/bindings.ts`.
#[cfg(any(debug_assertions, test))]
fn typescript_bindings(builder: &Builder<tauri::Wry>) -> String {
    let generated = builder
        .export_str(Typescript::default().bigint(BigIntExportBehavior::Number))
        .expect("Failed to generate typescript bindings");
    normalize_generated_bindings_source(&generated)
        .expect("Failed to normalize typescript binding errors")
}

/// Starts the Tauri application and shuts down transcription on process exit.
pub fn run(cli_args: CliArgs) {
    let console_filter = build_console_filter();

    let specta_builder = specta_builder();

    #[cfg(debug_assertions)]
    std::fs::write("../src/bindings.ts", typescript_bindings(&specta_builder))
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    // The headless path must run as its own instance (see the single-instance
    // note below), not forward to an already-running app.
    let headless_mode = cli_args.transcribe_file.is_some();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace)
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // In headless mode stdout carries only the result (JSON or
                    // plain), so console logs go to stderr to keep stdout parseable.
                    Target::new(if headless_mode {
                        TargetKind::Stderr
                    } else {
                        TargetKind::Stdout
                    })
                    .filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("murmur".into()),
                    })
                    .filter(|metadata| {
                        let file_level = FILE_LOG_LEVEL.load(Ordering::Relaxed);
                        metadata.level() <= level_filter_from_u8(file_level)
                    }),
                    // Stream logs to the webview (via the `log://log` event) so the
                    // debug panel's live log viewer can show them in real time. Only
                    // active while debug mode is on (its sole consumer), and shares the
                    // file log level so the "Log Level" setting controls verbosity.
                    Target::new(TargetKind::Webview).filter(|metadata| {
                        WEBVIEW_LOG_STREAMING.load(Ordering::Relaxed)
                            && metadata.level()
                                <= level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed))
                    }),
                ])
                .build(),
        );

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    // Single-instance forwards CLI args to an already-running Murmur and exits.
    // That would make `--transcribe-file` a silent no-op whenever the app is
    // already open, so headless mode skips it and runs a standalone instance.
    if !headless_mode {
        // The plugin can call its callback before setup has initialized the
        // window, coordinator, and recording manager. Register this state
        // before the plugin so early remote actions have somewhere safe to go.
        builder = builder.manage(SingleInstanceActionQueue::default());
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let action = action_from_args(&args);
            if let Some(action) = app
                .state::<SingleInstanceActionQueue>()
                .enqueue_or_dispatch(action)
            {
                dispatch_single_instance_action(app, action);
            }
        }));
    }

    builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(cli_args.clone())
        .setup(move |app| {
            specta_builder.mount_events(app);
            // Before anything reads the settings, which would take a damaged
            // file for a first launch and save the defaults over it.
            let settings_recovery = settings::recover_damaged_store(app.handle());

            // Headless one-shot path (`--transcribe-file`): only the transcription
            // manager is created, then a worker thread transcribes and exits. It
            // skips the window, tray, overlay, audio recorder (so it never opens
            // the mic, even with always_on_microphone), signal handlers, and
            // autostart that initialize_core_logic sets up.
            if headless_mode {
                let app_handle = app.handle().clone();
                let transcription_manager = Arc::new(TranscriptionManager::new(&app_handle));
                app_handle.manage(transcription_manager);

                let handle = app_handle.clone();
                let args = cli_args.clone();
                std::thread::spawn(move || {
                    let code = run_headless_guarded(|| run_headless_transcription(&handle, &args));
                    handle
                        .state::<Arc<TranscriptionManager>>()
                        .inner()
                        .shutdown();
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(code);
                });
                return Ok(());
            }

            #[cfg_attr(target_os = "windows", allow(unused_variables))]
            let main_window =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Murmur")
                    .inner_size(SETTINGS_WINDOW_SIZE.0, SETTINGS_WINDOW_SIZE.1)
                    .min_inner_size(SETTINGS_WINDOW_MIN_SIZE.0, SETTINGS_WINDOW_MIN_SIZE.1)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false)
                    .build()?;

            #[cfg(target_os = "macos")]
            prepare_settings_window_for_active_space(&main_window)?;

            let mut settings = get_settings(app.handle());

            // Apply the persisted appearance theme to the native title bar before
            // the window is shown, so it matches the in-app palette without a flash
            // of the wrong theme.
            shortcut::apply_window_theme(app.handle(), settings.theme);

            if let Err(error) = accent::apply_native_accent(app.handle(), settings.accent_color) {
                log::warn!("Failed to apply native accent icon: {error}");
            }

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            // Only forward logs to the webview while debug mode is on (the live log
            // viewer is the sole consumer and only exists in debug mode). This also
            // honors the runtime `--debug` override applied to `settings` above.
            WEBVIEW_LOG_STREAMING.store(settings.debug_mode, Ordering::Relaxed);
            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));

            initialize_core_logic(&app_handle);
            if let Some(recovery) = settings_recovery {
                user_alert::show(&app_handle, user_alert::Alert::SettingsRecovered(recovery));
            }

            // Populate the overlay-enabled cache from initial settings so the
            // audio path (overlay::emit_levels, called ~24 Hz during recording)
            // can do a single atomic load instead of reading the Tauri store.
            // Kept in sync by shortcut::change_overlay_style_setting.
            overlay::update_overlay_enabled_cache(
                settings.overlay_style != settings::OverlayStyle::None,
            );

            if cli_args.no_tray {
                tray::set_tray_visibility(&app_handle, false);
            }

            let should_hide = settings.start_hidden || cli_args.start_hidden;

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            // An update relaunch also shows it: the user just clicked Install.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            let update_relaunch = app
                .path()
                .app_data_dir()
                .is_ok_and(|dir| take_update_relaunch_marker(&dir));
            if !should_hide || !tray_available || update_relaunch {
                show_main_window(&app_handle);
            }

            // This is deliberately the final non-headless setup step. A remote
            // Toggle now observes the initialized overlay cache and final tray
            // visibility. A remote Show runs after the start-hidden decision,
            // so it always brings the window forward.
            app_handle
                .state::<SingleInstanceActionQueue>()
                .mark_ready_and_drain(|action| {
                    dispatch_single_instance_action(&app_handle, action)
                });

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _res = window.hide();

                #[cfg(target_os = "macos")]
                {
                    let settings = get_settings(window.app_handle());
                    let tray_visible =
                        settings.show_tray_icon && !window.app_handle().state::<CliArgs>().no_tray;
                    if tray_visible {
                        accent::set_dock_presence(window.app_handle(), accent::DockPresence::Hidden);
                    }
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                utils::refresh_tray_icon(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match &event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                show_main_window(app);
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    if !coordinator.allows_exit() {
                        // A cancelled, already-posted Cmd+V still needs its
                        // queued modifier-release receipt. Keep AppKit alive
                        // for the coordinator's bounded foreground drain.
                        api.prevent_exit();
                        coordinator.begin_shutdown(app.clone());
                    }
                }
            }
            tauri::RunEvent::Exit => {
                // Cmd+Q, the Dock's Quit and a logout skip `ExitRequested` and
                // so the coordinator's cancel, and macOS keeps the mute. A
                // dictation may still start and become ready while a meeting
                // closes below, so the mute is lifted for good.
                if let Some(audio) = app.try_state::<Arc<AudioRecordingManager>>() {
                    audio.invalidate_recording_readiness();
                    audio.remove_mute_for_good();
                }
                paste_tx::settle_pending();
                // Close a meeting's tracks so it is kept, not left interrupted.
                if let Some(manager) = app.try_state::<Arc<meetings::MeetingsManager>>() {
                    manager.shutdown();
                }
                if let Some(manager) = app.try_state::<Arc<TranscriptionManager>>() {
                    manager.shutdown();
                }
                // They skip the coordinator's drain too, which waits for a
                // token refresh: one cut off would sign every Codex client
                // out. A refresh request times out on its own within this.
                if !codex_auth::wait_for_refreshes(codex_auth::REFRESH_TIMEOUT) {
                    log::warn!("Timed out waiting for a Codex token refresh while quitting");
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod headless_guard_tests {
    use super::{
        leave_update_relaunch_marker, normalize_generated_bindings_source, run_headless_guarded,
        take_update_relaunch_marker,
    };

    /// AppKit's settings window placement.
    #[cfg(target_os = "macos")]
    mod settings_window {
    use super::super::{
        centered_window_origin, fitted_window_size, screen_containing,
        settings_window_collection_behavior, settings_window_needs_order_out,
        SETTINGS_WINDOW_MIN_SIZE, SETTINGS_WINDOW_SIZE,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    #[test]
    fn settings_window_moves_to_active_space_without_joining_every_space() {
        use objc2_app_kit::NSWindowCollectionBehavior;

        let current =
            NSWindowCollectionBehavior::Managed | NSWindowCollectionBehavior::CanJoinAllSpaces;
        let configured = settings_window_collection_behavior(current);

        assert!(configured.contains(NSWindowCollectionBehavior::Managed));
        assert!(configured.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(!configured.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
    }

    #[test]
    fn settings_window_is_reordered_only_when_visible_on_another_space() {
        assert!(settings_window_needs_order_out(true, false));
        assert!(!settings_window_needs_order_out(false, false));
        assert!(!settings_window_needs_order_out(true, true));
        assert!(!settings_window_needs_order_out(false, true));
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    #[test]
    fn settings_window_is_centered_in_the_visible_frame() {
        // A 1512x982 screen whose visible frame drops a 33-point menu bar and
        // a 70-point Dock. The vertical center is the visible frame's, not the
        // slightly higher spot NSWindow.center() picks.
        let visible = rect(0.0, 70.0, 1512.0, 879.0);
        let origin = centered_window_origin(visible, frame(SETTINGS_WINDOW_SIZE));
        assert_eq!((origin.x, origin.y), (366.0, 135.0));
    }

    #[test]
    fn settings_window_is_centered_on_a_secondary_screen() {
        // A screen left of the main one has negative AppKit coordinates.
        let visible = rect(-1920.0, -200.0, 1920.0, 1055.0);
        let origin = centered_window_origin(visible, frame(SETTINGS_WINDOW_SIZE));
        assert_eq!((origin.x, origin.y), (-1350.0, -47.0));
    }

    #[test]
    fn a_window_larger_than_the_screen_keeps_its_title_bar_visible() {
        let visible = rect(0.0, 0.0, 600.0, 500.0);
        let size = frame(SETTINGS_WINDOW_SIZE);
        let origin = centered_window_origin(visible, size);
        // Left edge and top edge stay inside the visible frame.
        assert_eq!((origin.x, origin.y + size.height), (0.0, 500.0));
    }

    /// The frame size of a content size, with the 28-point title bar.
    fn frame(content: (f64, f64)) -> NSSize {
        NSSize::new(content.0, content.1 + 28.0)
    }

    #[test]
    fn settings_window_keeps_its_default_size_on_a_large_screen() {
        let visible = NSSize::new(1512.0, 879.0);
        let size = fitted_window_size(
            visible,
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, frame(SETTINGS_WINDOW_SIZE));
    }

    #[test]
    fn settings_window_shrinks_to_a_small_screen() {
        // A 13" 1280x800 screen less a 25-point menu bar and a 70-point Dock.
        let visible = NSSize::new(1280.0, 705.0);
        let size = fitted_window_size(
            visible,
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, NSSize::new(SETTINGS_WINDOW_SIZE.0, 705.0));
    }

    #[test]
    fn settings_window_never_shrinks_below_its_minimum() {
        let visible = NSSize::new(600.0, 500.0);
        let size = fitted_window_size(
            visible,
            frame(SETTINGS_WINDOW_SIZE),
            frame(SETTINGS_WINDOW_MIN_SIZE),
        );
        assert_eq!(size, frame(SETTINGS_WINDOW_MIN_SIZE));
    }

    #[test]
    fn the_screen_under_the_pointer_is_the_one_containing_it() {
        let screens = [
            rect(0.0, 0.0, 1512.0, 982.0),
            rect(1512.0, -100.0, 1920.0, 1080.0),
        ];
        assert_eq!(
            screen_containing(&screens, NSPoint::new(2000.0, 500.0)),
            Some(1)
        );
        assert_eq!(
            screen_containing(&screens, NSPoint::new(10.0, 982.0)),
            Some(0)
        );
        assert_eq!(screen_containing(&screens, NSPoint::new(-5.0, 10.0)), None);
    }
    }

    #[test]
    fn an_update_relaunch_shows_the_window_once() {
        let dir = tempfile::tempdir().unwrap();
        let app_data_dir = dir.path().join("not-created-yet");
        assert!(!take_update_relaunch_marker(&app_data_dir));

        leave_update_relaunch_marker(&app_data_dir).unwrap();
        assert!(take_update_relaunch_marker(&app_data_dir));
        // The next start, at login for instance, follows "Start hidden" again.
        assert!(!take_update_relaunch_marker(&app_data_dir));
    }

    /// `MURMUR_WRITE_BINDINGS=1 cargo test bindings_are_current` rewrites the
    /// file after a command or event changes, without launching the app.
    #[test]
    fn bindings_are_current() {
        let generated = super::typescript_bindings(&super::specta_builder());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");
        if std::env::var_os("MURMUR_WRITE_BINDINGS").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        assert!(
            std::fs::read_to_string(path).unwrap() == generated,
            "src/bindings.ts is stale: run with MURMUR_WRITE_BINDINGS=1"
        );
    }

    #[test]
    fn preserves_normal_exit_codes() {
        assert_eq!(run_headless_guarded(|| 2), 2);
    }

    #[test]
    fn converts_worker_panics_to_runtime_failures() {
        assert_eq!(run_headless_guarded(|| panic!("simulated failure")), 1);
    }

    #[test]
    fn generated_result_errors_are_normalized_to_strings() {
        let generated = concat!(
            "async command(): Promise<Result<null, string>> { ",
            "else return { status: \"error\", error: e\t as   any }; }",
        );
        let normalized = normalize_generated_bindings_source(generated).unwrap();
        assert_eq!(
            normalized,
            concat!(
                "async command(): Promise<Result<null, string>> { ",
                r#"else return { status: "error", error: String(e) }; }"#,
            )
        );
    }

    #[test]
    fn generated_enum_errors_keep_their_type() {
        let generated = concat!(
            "async first(): Promise<Result<Record<string, Foo>, string>> { error: e as any }\n",
            "async second(): Promise<Result<null, SignInError>> { error: e as any }",
        );
        assert_eq!(
            normalize_generated_bindings_source(generated).unwrap(),
            concat!(
                "async first(): Promise<Result<Record<string, Foo>, string>> { error: String(e) }\n",
                "async second(): Promise<Result<null, SignInError>> { error: e as SignInError }",
            )
        );
    }

    #[test]
    fn unrecognized_generated_result_wrapper_fails_loudly() {
        let generated = "async command(): Promise<Result<null, string>> { error: e as unknown }";
        assert!(normalize_generated_bindings_source(generated).is_err());
    }

    #[test]
    fn mixed_generated_result_wrappers_fail_loudly() {
        let generated = concat!(
            "async first(): Promise<Result<null, string>> { error: e as any }",
            "async second(): Promise<Result<null, string>> { error: e as unknown }",
        );
        assert!(normalize_generated_bindings_source(generated).is_err());
    }
}

#[cfg(test)]
mod exit_tests {
    /// The run-loop closure needs a live AppKit event loop, so these tests
    /// pin the calls of its `RunEvent::Exit` arm instead.
    fn exit_arm() -> &'static str {
        include_str!("lib.rs")
            .split("tauri::RunEvent::Exit => {")
            .nth(1)
            .and_then(|rest| rest.split("_ => {}").next())
            .expect("the run loop handles RunEvent::Exit")
    }

    /// The promised transcript dies with the process, and only the exiting
    /// process can still put the user's clipboard back.
    #[test]
    fn quitting_settles_the_pending_paste() {
        assert!(exit_arm().contains("paste_tx::settle_pending()"));
    }

    /// Cmd+Q, the Dock's Quit and a logout end in `Exit` without
    /// `ExitRequested`, so only `Exit` can lift "mute while recording":
    /// macOS keeps the output mute after the process is gone.
    #[test]
    fn quitting_restores_the_mute() {
        let exit_arm = exit_arm();
        // A dictation started while a meeting is being closed below still
        // reaches its mute step with a current recording, so the restore
        // must refuse every later mute.
        let restore = exit_arm
            .find(".remove_mute_for_good()")
            .expect("quitting restores the mute for good");
        let invalidate = exit_arm
            .find(".invalidate_recording_readiness()")
            .expect("quitting invalidates a dictation's pending mute");
        assert!(invalidate < restore);
    }

    /// Only the tray's Quit goes through `ExitRequested`, whose drain waits
    /// for a Codex token refresh. A refresh cut off by the exit has spent the
    /// old refresh token without saving the new one, which signs every Codex
    /// client out.
    #[test]
    fn quitting_waits_for_a_token_refresh() {
        let exit_arm = exit_arm();
        let wait = exit_arm
            .find("codex_auth::wait_for_refreshes(")
            .expect("quitting waits for a token refresh");
        // Once the managers have cancelled the calls that could start one.
        let shutdown = exit_arm
            .rfind(".shutdown()")
            .expect("quitting shuts the managers down");
        assert!(shutdown < wait);
    }
}
