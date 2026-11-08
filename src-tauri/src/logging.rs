use super::*;
use env_filter::Builder as EnvFilterBuilder;
use std::sync::atomic::{AtomicBool, AtomicU8};

pub static FILE_LOG_LEVEL: AtomicU8 = AtomicU8::new(log::LevelFilter::Info as u8);

/// When `true`, log records are also forwarded to the webview through the
/// `log://log` event for the debug panel's live log viewer. It follows debug
/// mode, because that viewer is the only consumer and only exists in debug
/// mode, so normal runs never broadcast log records (which can include file
/// paths and diagnostics) onto the frontend event bus. Synced at startup
/// and whenever debug mode is toggled (see `shortcut::change_debug_mode_setting`).
pub static WEBVIEW_LOG_STREAMING: AtomicBool = AtomicBool::new(false);

/// Dependencies whose Debug lines drown the app's own (connection setup,
/// codec probing, the whole update response with its release notes). They log
/// only warnings and errors unless the level is Trace.
pub(crate) const QUIET_DEPENDENCIES: &[&str] = &[
    "reqwest",
    "hyper",
    "h2",
    "symphonia",
    "enigo",
    "rusqlite_migration",
    "tauri_plugin_updater",
];

/// A crate is matched by its root module, and its sibling crates by name prefix
/// (`hyper_util`, `symphonia_core`, `symphonia_bundle_mp3`).
pub(crate) fn is_quiet_dependency(target: &str) -> bool {
    let root = target.split("::").next().unwrap_or(target);
    QUIET_DEPENDENCIES.iter().any(|name| {
        root == *name
            || root
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('_'))
    })
}

/// Whether the file log and the live log viewer keep a line.
pub(crate) fn log_line_enabled(metadata: &log::Metadata, configured: log::LevelFilter) -> bool {
    if metadata.level() > configured {
        return false;
    }
    configured >= log::LevelFilter::Trace
        || metadata.level() <= log::Level::Warn
        || !is_quiet_dependency(metadata.target())
}

/// The level the file log runs at. A new install starts at Info; the old
/// default was Debug, which a user who never opened the Debug panel (where
/// the level is chosen) did not pick, so without debug mode a stored Debug
/// means Info. Turning debug mode on brings back the stored level.
pub(crate) fn effective_log_level(
    debug_mode: bool,
    stored: settings::LogLevel,
) -> settings::LogLevel {
    if !debug_mode && stored == settings::LogLevel::Debug {
        settings::LogLevel::Info
    } else {
        stored
    }
}

/// Applies `effective_log_level` to the running file log.
pub(crate) fn sync_file_log_level(debug_mode: bool, stored: settings::LogLevel) {
    let level: tauri_plugin_log::LogLevel = effective_log_level(debug_mode, stored).into();
    let level: log::Level = level.into();
    FILE_LOG_LEVEL.store(level.to_level_filter() as u8, Ordering::Relaxed);
}

pub(crate) fn level_filter_from_u8(value: u8) -> log::LevelFilter {
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

pub(crate) fn build_console_filter() -> env_filter::Filter {
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
