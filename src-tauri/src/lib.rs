#[cfg(not(any(target_os = "macos", target_os = "windows")))]
compile_error!("Murmur supports macOS and Windows only.");

mod accent;
#[cfg(target_os = "macos")]
mod accessibility;
mod actions;
mod app_windows;
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
#[cfg(target_os = "windows")]
pub mod console_windows;
#[cfg(target_os = "windows")]
mod data_dirs_windows;
mod dictation_gesture;
mod durable;
mod engine;
mod field_excerpt;
#[cfg(target_os = "windows")]
mod foreground;
mod headless;
mod helpers;
mod http;
mod input;
mod insights;
mod leading_space;
pub mod local_models;
mod logging;
mod managers;
pub mod mcp;
mod meetings;
mod operation;
mod overlay;
mod panic_log;
mod paste_last;
mod paste_tx;
mod recently_cancelled;
mod runtime;
mod search;
mod settings;
mod settings_window;
mod shortcut;
mod signal_handle;
mod single_instance_actions;
mod transcription_coordinator;
mod transcription_error;
mod tray;
mod tray_i18n;
#[cfg(target_os = "windows")]
mod uia;
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

use managers::audio::AudioRecordingManager;
use managers::history::{DatabaseStartup, HistoryManager};
use managers::transcription::TranscriptionManager;
use std::sync::atomic::Ordering;
use std::sync::Arc;
pub use transcription_coordinator::TranscriptionCoordinator;

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_log::{Builder as LogBuilder, RotationStrategy, Target, TargetKind};

use crate::settings::get_settings;
use crate::single_instance_actions::{action_from_args, SingleInstanceActionQueue};
#[cfg(target_os = "macos")]
use app_windows::prepare_settings_window_for_active_space;
use app_windows::{build_main_window, dispatch_single_instance_action, show_main_window};
#[cfg(target_os = "windows")]
use app_windows::{fitted_window_size, remember_settings_window_size};
use headless::{run_headless_guarded, run_headless_transcription};
pub(crate) use logging::sync_file_log_level;
use logging::{build_console_filter, level_filter_from_u8, log_line_enabled};
pub use logging::{FILE_LOG_LEVEL, WEBVIEW_LOG_STREAMING};

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

fn initialize_core_logic(app_handle: &AppHandle) {
    #[cfg(target_os = "windows")]
    data_dirs_windows::prepare(app_handle);

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
    app_handle.manage(Arc::new(recently_cancelled::RecentlyCancelled::default()));
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
                    // A window built here is not listening yet: it takes
                    // the check once it is.
                    settings_window::queue(app, settings_window::MainAction::CheckUpdates);
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
            tray::RESTORE_CANCELLED_ITEM_ID => {
                recently_cancelled::request_restore(app);
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

fn mark_update_relaunch(app: &AppHandle) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    if let Err(e) = leave_update_relaunch_marker(&app_data_dir) {
        // The update is installed either way; only the window stays hidden.
        log::warn!("Failed to mark the update relaunch: {e}");
    }
    Ok(())
}

/// Restarts into the installed update and shows the settings window again.
#[tauri::command]
#[specta::specta]
fn relaunch_after_update(app: AppHandle) -> Result<(), String> {
    mark_update_relaunch(&app)?;
    app.request_restart();
    Ok(())
}

/// Run before an update installs: on Windows the updater plugin ends the
/// process once the installer starts, so the exit sequence never runs.
/// Leaves the relaunch marker and writes the settings kept in memory.
#[tauri::command]
#[specta::specta]
fn prepare_update_install(app: AppHandle) {
    if let Err(e) = mark_update_relaunch(&app) {
        log::warn!("Failed to prepare the update install: {e}");
    }
    settings::flush_pending_save(&app);
}

/// Every command and event the webview can use.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            shortcut::change_binding,
            shortcut::reset_binding,
            shortcut::get_shortcut_registration_errors,
            shortcut::get_key_labels,
            shortcut::get_recorded_key_names,
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
            shortcut::change_leading_space_setting,
            shortcut::change_append_trailing_space_setting,
            shortcut::change_lazy_stream_close_setting,
            commands::cleanup::change_ai_cleanup_setting,
            commands::cleanup::change_dictation_style_setting,
            commands::cleanup::change_category_styles_setting,
            commands::cleanup::change_window_names_setting,
            commands::cleanup::change_whisper_mode_setting,
            commands::cleanup::change_cleanup_short_dictations_setting,
            shortcut::change_app_language_setting,
            shortcut::change_update_checks_setting,
            shortcut::change_show_whats_new_on_update_setting,
            shortcut::change_whats_new_last_seen_version_setting,
            shortcut::change_show_tray_icon_setting,
            show_main_window_command,
            settings_window::report_settings_view,
            settings_window::report_opened_meeting,
            settings_window::get_last_settings_section,
            settings_window::hold_settings_window,
            settings_window::release_settings_window,
            settings_window::take_main_action,
            settings_window::take_meetings_action,
            relaunch_after_update,
            prepare_update_install,
            updates::get_update_status,
            updates::check_for_updates,
            commands::cancel_operation,
            overlay::copy_notice_again,
            overlay::meeting_notice::copy_meeting_notice,
            overlay::meeting_notice::open_meeting_notice,
            overlay::fit_copy_notice,
            recently_cancelled::restore_cancelled_dictation,
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
            commands::history::search_history,
            commands::history::toggle_history_entry_saved,
            commands::history::get_audio_file_path,
            commands::history::delete_history_entry,
            commands::history::clear_history,
            commands::history::retry_history_entry_transcription,
            commands::history::get_retranscribe_languages,
            commands::history::get_history_versions,
            commands::history::restore_history_version,
            commands::history::update_history_limit,
            commands::history::update_recording_retention_period,
            helpers::clamshell::is_laptop,
            insights::get_insights,
            insights::share::copy_insights_card,
            insights::share::save_insights_card,
            commands::meetings::get_meeting_runtime,
            commands::meetings::start_meeting,
            commands::meetings::stop_meeting,
            commands::meetings::discard_meeting,
            commands::meetings::get_meetings,
            commands::meetings::search_meetings,
            commands::meetings::delete_meeting,
            commands::meetings::get_meeting,
            commands::meetings::rename_meeting,
            commands::meetings::rename_speaker,
            commands::meetings::retry_meeting,
            commands::meetings::reanalyze_meeting,
            commands::meetings::import_meeting_audio,
            commands::meetings::cancel_meeting_import,
            commands::meetings::get_meeting_import,
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
            commands::mcp::change_mcp_enabled_setting,
            commands::mcp::change_mcp_include_dictations_setting,
            commands::mcp::get_mcp_setup,
            commands::meetings::answer_call_prompt,
            overlay::fit_call_prompt,
            overlay::get_call_prompt,
            commands::meetings::change_meeting_live_transcript_setting,
            commands::meetings::get_live_transcript,
        ])
        .events(collect_events![
            managers::history::HistoryUpdatePayload,
            insights::InsightsUpdated,
            meetings::MeetingStateChanged,
            meetings::MeetingsUpdated,
            meetings::import_audio::MeetingImportChanged,
            meetings::processor::MeetingProgress,
            local_models::LocalModelStatus,
            local_models::SwitchedToLocal,
            updates::UpdateStatus,
            meetings::live::MeetingLiveUpdated
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
    runtime::install();
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
                        log_line_enabled(metadata, level_filter_from_u8(file_level))
                    }),
                    // Stream logs to the webview (via the `log://log` event) so the
                    // debug panel's live log viewer can show them in real time. Only
                    // active while debug mode is on (its sole consumer), and shares the
                    // file log level so the "Log Level" setting controls verbosity.
                    Target::new(TargetKind::Webview).filter(|metadata| {
                        WEBVIEW_LOG_STREAMING.load(Ordering::Relaxed)
                            && log_line_enabled(
                                metadata,
                                level_filter_from_u8(FILE_LOG_LEVEL.load(Ordering::Relaxed)),
                            )
                    }),
                ])
                .build(),
        );
    panic_log::install();

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
                    settings::flush_pending_save(&handle);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    let _ = std::io::stderr().flush();
                    std::process::exit(code);
                });
                return Ok(());
            }

            app.manage(settings_window::SettingsWindow::default());
            // Built even for a hidden start: its page runs the onboarding
            // checks and starts the shortcuts, then Rust releases it.
            let main_window = build_main_window(app.handle())?;

            #[cfg(target_os = "macos")]
            prepare_settings_window_for_active_space(&main_window)?;
            #[cfg(not(target_os = "macos"))]
            drop(main_window);

            let mut settings = get_settings(app.handle());

            // A hidden Dock tile has nothing to draw the icon into, and the
            // default accent is the bundle's own icon: only a chosen accent
            // costs the recolor and encode.
            if let Err(error) = accent::restore_accent_at_launch(
                app.handle(),
                settings.accent_color,
                settings.start_hidden && settings.show_tray_icon,
            ) {
                log::warn!("Failed to apply native accent icon: {error}");
            }

            // CLI --debug flag overrides debug_mode and log level (runtime-only, not persisted)
            if cli_args.debug {
                settings.debug_mode = true;
                settings.log_level = settings::LogLevel::Trace;
            }

            sync_file_log_level(settings.debug_mode, settings.log_level);
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

            // scripts/smoke-macos-app.sh waits for this line before a release
            // is published.
            log::info!("Setup complete (version {})", env!("CARGO_PKG_VERSION"));

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let app = window.app_handle();
                // A destroyed settings window takes its web process with it.
                let destroy = window.label() == settings_window::MAIN_WINDOW
                    && settings_window::current_close_action(app)
                        == settings_window::CloseAction::Destroy;
                if !destroy {
                    api.prevent_close();
                    let _res = window.hide();
                }
                #[cfg(target_os = "windows")]
                if destroy {
                    remember_settings_window_size(window);
                }

                #[cfg(target_os = "macos")]
                if settings_window::tray_visible(app) {
                    accent::set_dock_presence(app, accent::DockPresence::Hidden);
                }
            }
            tauri::WindowEvent::Destroyed if window.label() == settings_window::MAIN_WINDOW => {
                settings_window::on_destroyed(window.app_handle());
                // A page closed while a field captured keys cannot resume the
                // hotkeys it suspended. The registrations wait on the main
                // thread, which runs this handler.
                #[cfg(target_os = "windows")]
                {
                    let app = window.app_handle().clone();
                    std::thread::spawn(move || shortcut::resume_bindings_if_suspended(&app));
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
            // No exit code: the last window went. Murmur lives in the menu
            // bar, and the settings window is destroyed on close.
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => {
                api.prevent_exit();
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
                // The managers below cancel running dictations, which then try
                // to keep their audio: the closed slot refuses. The tray's
                // Quit closed it already; Cmd+Q never began that shutdown.
                recently_cancelled::close(app);
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
                // A settings change is written in the background.
                settings::flush_pending_save(app);
            }
            _ => {}
        });
}

#[cfg(test)]
#[path = "lib/log_filter_tests.rs"]
mod log_filter_tests;

#[cfg(test)]
#[path = "lib/headless_guard_tests.rs"]
mod headless_guard_tests;

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

    /// A cancelled dictation lives in memory only, and is zeroed before the
    /// process goes. A closed slot refuses a keep, so the order against the
    /// managers' cancel does not matter.
    #[test]
    fn quitting_closes_the_cancelled_dictation() {
        assert!(exit_arm().contains("recently_cancelled::close(app)"));
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

    /// A setting changed just before quitting may still be on its way to
    /// disk in the background; the exit would cut that write off.
    #[test]
    fn quitting_writes_pending_settings() {
        assert!(exit_arm().contains("settings::flush_pending_save(app)"));
    }
}
