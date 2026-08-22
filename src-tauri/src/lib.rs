mod actions;
mod audio_feedback;
pub mod audio_toolkit;
mod autostart;
pub mod cli;
mod clipboard;
mod codex_transcribe;
mod commands;
mod helpers;
mod input;
mod managers;
mod memory;
mod overlay;
mod paste_tx;
pub mod portable;
mod settings;
mod shortcut;
mod signal_handle;
mod transcription_coordinator;
mod tray;
mod tray_i18n;
mod utils;

pub use cli::CliArgs;
#[cfg(debug_assertions)]
use specta_typescript::{BigIntExportBehavior, Typescript};
use tauri_specta::{collect_commands, collect_events, Builder};

use env_filter::Builder as EnvFilterBuilder;
use managers::audio::AudioRecordingManager;
use managers::history::HistoryManager;
use managers::transcription::TranscriptionManager;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use tauri::image::Image;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;

#[cfg(debug_assertions)]
fn normalize_generated_bindings(path: &std::path::Path) -> std::io::Result<()> {
    let generated = std::fs::read_to_string(path)?;
    let normalized = normalize_generated_bindings_source(&generated);
    std::fs::write(path, normalized)
}

#[cfg(debug_assertions)]
fn normalize_generated_bindings_source(generated: &str) -> String {
    generated.replace("error: e  as any", "error: String(e)")
}

// Global atomic to store the file log level filter
// We use u8 to store the log::LevelFilter as a number
pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Debug as u8);

/// When `true`, log records are also forwarded to the webview via the
/// `log://log` event for the debug panel's live log viewer. Gated on debug
/// mode — the live log viewer is its only consumer and only exists in debug
/// mode — so normal runs never broadcast log records (which can include file
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

fn show_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        if let Err(e) = main_window.unminimize() {
            log::error!("Failed to unminimize webview window: {}", e);
        }
        if let Err(e) = main_window.show() {
            log::error!("Failed to show webview window: {}", e);
        }
        if let Err(e) = main_window.set_focus() {
            log::error!("Failed to focus webview window: {}", e);
        }
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log::error!("Failed to set activation policy to Regular: {}", e);
            }
        }
        return;
    }

    let webview_labels = app.webview_windows().keys().cloned().collect::<Vec<_>>();
    log::error!(
        "Main window not found. Webview labels: {:?}",
        webview_labels
    );
}

#[allow(unused_variables)]
fn should_force_show_permissions_window(app: &AppHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        let status = commands::audio::get_windows_microphone_permission_status();
        if status.supported && status.overall_access == commands::audio::PermissionAccess::Denied {
            log::info!(
                "Windows microphone permissions are denied; forcing main window visible for onboarding"
            );
            return true;
        }
    }

    false
}

fn initialize_core_logic(app_handle: &AppHandle) {
    // Note: Enigo (keyboard/mouse simulation) is NOT initialized here.
    // The frontend is responsible for calling the `initialize_enigo` command
    // after onboarding completes. This avoids triggering permission dialogs
    // on macOS before the user is ready.

    // Initialize the managers.
    let transcription_manager = Arc::new(TranscriptionManager::new(app_handle));
    let recording_manager = Arc::new(
        AudioRecordingManager::new(app_handle).expect("Failed to initialize recording manager"),
    );
    let history_manager =
        Arc::new(HistoryManager::new(app_handle).expect("Failed to initialize history manager"));

    app_handle.manage(recording_manager.clone());
    app_handle.manage(transcription_manager.clone());
    app_handle.manage(history_manager.clone());
    app_handle.manage(tray::CurrentTrayIconState::new());

    // Note: Shortcuts are NOT initialized here.
    // The frontend is responsible for calling the `initialize_shortcuts` command
    // after permissions are confirmed (on macOS) or after onboarding completes.
    // This matches the pattern used for Enigo initialization.

    // Set up signal handlers for toggling transcription. On Linux, SIGUSR1 is
    // deliberately not handled — it belongs to WebKitGTK's garbage collector
    // (#1660) — see signal_handle.rs.
    #[cfg(unix)]
    signal_handle::setup_signal_handler(app_handle.clone());

    // Apply macOS Accessory policy if starting hidden and tray is available.
    // If the tray icon is disabled, keep the dock icon so the user can reopen.
    #[cfg(target_os = "macos")]
    {
        let settings = settings::get_settings(app_handle);
        if settings.start_hidden && settings.show_tray_icon {
            let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    // Get the current theme to set the appropriate initial icon
    let initial_theme = tray::get_current_theme(app_handle);

    // Choose the appropriate initial icon based on theme
    let initial_icon_path = tray::get_icon_path(initial_theme, tray::TrayIconState::Idle);

    let mut tray_builder = TrayIconBuilder::new()
        .icon(
            Image::from_path(
                app_handle
                    .path()
                    .resolve(initial_icon_path, tauri::path::BaseDirectory::Resource)
                    .unwrap(),
            )
            .unwrap(),
        )
        .tooltip(tray::tray_tooltip())
        .icon_as_template(true);

    // Windows notification-area convention: left click opens the app, right click
    // shows the menu. Elsewhere (macOS menu bar, Linux) the menu stays on left click.
    #[cfg(target_os = "windows")]
    {
        tray_builder = tray_builder
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                let opens_window = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if opens_window {
                    show_main_window(tray.app_handle());
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        tray_builder = tray_builder.show_menu_on_left_click(true);
    }

    let tray = tray_builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
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
            "cancel" => {
                use crate::utils::cancel_current_operation;

                cancel_current_operation(app);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app_handle)
        .unwrap();
    app_handle.manage(tray);

    // Initialize tray menu with idle state
    utils::update_tray_menu(app_handle, None);

    // Apply show_tray_icon setting
    let settings = settings::get_settings(app_handle);
    if !settings.show_tray_icon {
        tray::set_tray_visibility(app_handle, false);
    }

    // Apply the autostart preference (SMAppService login item on macOS 13+,
    // tauri-plugin-autostart elsewhere)
    autostart::apply_autostart(app_handle, settings.autostart_enabled);

    // Create the recording overlay window (hidden by default)
    utils::create_recording_overlay(app_handle);
}

#[tauri::command]
#[specta::specta]
fn trigger_update_check(app: AppHandle) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    if !settings.update_checks_enabled {
        return Ok(());
    }
    app.emit("check-for-updates", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
fn show_main_window_command(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
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

    // read_wav_samples reads 16-bit int samples and does no validation; the app
    // only ever saves 16 kHz mono 16-bit PCM, so reject anything else rather than
    // transcribe garbage / mis-time / mis-decode.
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
    let text = match tm.transcribe(samples) {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(cli_args: CliArgs) {
    memory::init_allocator();

    // Detect portable mode before anything else
    portable::init();

    // Parse console logging directives from RUST_LOG, falling back to info-level logging
    // when the variable is unset
    let console_filter = build_console_filter();

    let specta_builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::change_ptt_setting,
            shortcut::change_audio_feedback_setting,
            shortcut::change_audio_feedback_volume_setting,
            shortcut::change_sound_theme_setting,
            shortcut::change_theme_setting,
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
            shortcut::get_available_typing_tools,
            shortcut::change_typing_tool_setting,
            shortcut::change_external_script_path_setting,
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
            shortcut::change_filler_word_removal_enabled_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_show_whats_new_on_update_setting,
            shortcut::change_whats_new_last_seen_version_setting,
            shortcut::change_show_tray_icon_setting,
            trigger_update_check,
            show_main_window_command,
            commands::cancel_operation,
            commands::is_portable,
            commands::get_app_dir_path,
            commands::get_app_settings,
            commands::get_default_settings,
            commands::get_log_dir_path,
            commands::set_log_level,
            commands::open_recordings_folder,
            commands::open_log_dir,
            commands::open_app_data_dir,
            commands::initialize_enigo,
            commands::initialize_shortcuts,
            commands::audio::update_microphone_mode,
            commands::audio::get_microphone_mode,
            commands::audio::get_windows_microphone_permission_status,
            commands::audio::open_microphone_privacy_settings,
            commands::audio::get_available_microphones,
            commands::audio::set_selected_microphone,
            commands::audio::get_selected_microphone,
            commands::audio::get_available_output_devices,
            commands::audio::set_selected_output_device,
            commands::audio::get_selected_output_device,
            commands::audio::play_test_sound,
            commands::audio::check_custom_sounds,
            commands::audio::set_clamshell_microphone,
            commands::audio::get_clamshell_microphone,
            commands::audio::is_recording,
            commands::audio::get_microphone_channels,
            commands::audio::set_selected_channel,
            commands::transcription::get_codex_auth_status,
            commands::transcription::complete_onboarding,
            commands::history::get_history_entries,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::retry_history_entry_transcription,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            helpers::clamshell::is_laptop,
        ])
        .events(collect_events![managers::history::HistoryUpdatePayload]);

    #[cfg(debug_assertions)] // <- Only export on non-release builds
    specta_builder
        .export(
            Typescript::default()
                .bigint(BigIntExportBehavior::Number)
                .formatter(normalize_generated_bindings),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    let invoke_handler = specta_builder.invoke_handler();

    // The headless path must run as its own instance (see the single-instance
    // note below), not forward to an already-running app.
    let headless_mode = cli_args.transcribe_file.is_some();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .device_event_filter(tauri::DeviceEventFilter::Always)
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            LogBuilder::new()
                .level(log::LevelFilter::Trace) // Set to most verbose level globally
                .max_file_size(500_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .clear_targets()
                .targets([
                    // Console output respects RUST_LOG environment variable. In
                    // headless mode (--transcribe-file)
                    // stdout carries only the result (JSON or plain), so send console
                    // logs to stderr instead to keep stdout clean for CI parsing.
                    Target::new(if headless_mode {
                        TargetKind::Stderr
                    } else {
                        TargetKind::Stdout
                    })
                    .filter({
                        let console_filter = console_filter.clone();
                        move |metadata| console_filter.enabled(metadata)
                    }),
                    // File logs respect the user's settings (stored in FILE_LOG_LEVEL atomic)
                    Target::new(if let Some(data_dir) = portable::data_dir() {
                        TargetKind::Folder {
                            path: data_dir.join("logs"),
                            file_name: Some("murmur".into()),
                        }
                    } else {
                        TargetKind::LogDir {
                            file_name: Some("murmur".into()),
                        }
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
    // That would make the headless path
    // (`--transcribe-file`) a silent no-op whenever the
    // app is already open, so skip it in headless mode and run a standalone
    // instance instead.
    if !headless_mode {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--toggle-transcription") {
                signal_handle::send_transcription_input(app, "transcribe", "CLI");
            } else if args.iter().any(|a| a == "--cancel") {
                crate::utils::cancel_current_operation(app);
            } else {
                show_main_window(app);
            }
        }));
    }

    builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_macos_permissions::init())
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

            // Headless one-shot path (`--transcribe-file`): initialize only the
            // store/paths plugins and transcription manager, then run on a worker thread and
            // exit. Deliberately skips the window, tray, overlay, audio
            // recorder (so it never opens the mic, even with always_on_microphone),
            // signal handlers, and autostart that initialize_core_logic sets up.
            if headless_mode {
                let app_handle = app.handle().clone();
                let transcription_manager = Arc::new(TranscriptionManager::new(&app_handle));
                app_handle.manage(transcription_manager);

                let handle = app_handle.clone();
                let args = cli_args.clone();
                std::thread::spawn(move || {
                    let code = run_headless_guarded(|| run_headless_transcription(&handle, &args));
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(code);
                });
                return Ok(());
            }

            // Create main window programmatically so we can set data_directory
            // for portable mode (redirects WebView2 cache to portable Data dir)
            let mut win_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("/".into()))
                    .title("Murmur")
                    .inner_size(680.0, 570.0)
                    .min_inner_size(680.0, 570.0)
                    .resizable(true)
                    .maximizable(true)
                    .visible(false);

            if let Some(data_dir) = portable::data_dir() {
                win_builder = win_builder.data_directory(data_dir.join("webview"));
            }

            win_builder.build()?;

            let mut settings = get_settings(app.handle());

            // Apply the persisted appearance theme to the native title bar before
            // the window is shown, so it matches the in-app palette without a flash
            // of the wrong theme. See `apply_window_theme` for what this does per
            // platform.
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            shortcut::apply_window_theme(app.handle(), settings.theme);

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            let tauri_log_level: tauri_plugin_log::LogLevel = settings.log_level.into();
            let file_log_level: log::Level = tauri_log_level.into();
            // Store the file log level in the atomic for the filter to use
            FILE_LOG_LEVEL.store(file_log_level.to_level_filter() as u8, Ordering::Relaxed);
            // Only forward logs to the webview while debug mode is on (the live log
            // viewer is the sole consumer and only exists in debug mode). This also
            // honors the runtime `--debug` override applied to `settings` above.
            WEBVIEW_LOG_STREAMING.store(settings.debug_mode, Ordering::Relaxed);
            let app_handle = app.handle().clone();
            app.manage(TranscriptionCoordinator::new(app_handle.clone()));

            initialize_core_logic(&app_handle);

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

            // Show main window only if not starting hidden.
            // CLI --start-hidden flag overrides the setting.
            // But if permission onboarding is required, always show the window.
            let should_hide = settings.start_hidden || cli_args.start_hidden;
            let should_force_show = should_force_show_permissions_window(&app_handle);

            // If start_hidden but tray is disabled, we must show the window
            // anyway. Without a tray icon, the dock is the only way back in.
            let tray_available = settings.show_tray_icon && !cli_args.no_tray;
            if should_force_show || !should_hide || !tray_available {
                show_main_window(&app_handle);
            }

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
                        // Tray is available: hide the dock icon, app lives in the tray
                        let res = window
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                        if let Err(e) = res {
                            log::error!("Failed to set activation policy: {}", e);
                        }
                    }
                    // No tray: keep the dock icon visible so the user can reopen
                }
            }
            tauri::WindowEvent::ThemeChanged(theme) => {
                log::info!("Theme changed to: {:?}", theme);
                // Re-apply the current tray state with the new theme's icon set
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
            _ => {}
        });
}

#[cfg(test)]
mod headless_guard_tests {
    use super::{normalize_generated_bindings_source, run_headless_guarded};

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
        let generated = r#"else return { status: "error", error: e  as any };"#;
        let normalized = normalize_generated_bindings_source(generated);
        assert_eq!(
            normalized,
            r#"else return { status: "error", error: String(e) };"#
        );
    }
}
