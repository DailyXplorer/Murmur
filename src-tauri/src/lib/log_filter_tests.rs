use super::*;
use crate::logging::{effective_log_level, log_line_enabled};
use log::{Level, LevelFilter, Metadata};

fn enabled(target: &str, level: Level, configured: LevelFilter) -> bool {
    log_line_enabled(
        &Metadata::builder().target(target).level(level).build(),
        configured,
    )
}

#[test]
fn noisy_dependencies_log_warnings_only_below_trace() {
    for target in [
        "reqwest::connect",
        "hyper_util::client::legacy::connect::http",
        "symphonia_core::probe",
        "symphonia_bundle_mp3::demuxer",
        "hyper_util::client::legacy::pool",
        "enigo",
        "rusqlite_migration",
        "tauri_plugin_updater::updater",
    ] {
        assert!(
            !enabled(target, Level::Debug, LevelFilter::Debug),
            "{target}"
        );
        assert!(
            !enabled(target, Level::Info, LevelFilter::Debug),
            "{target}"
        );
        assert!(enabled(target, Level::Warn, LevelFilter::Debug), "{target}");
        assert!(enabled(target, Level::Error, LevelFilter::Info), "{target}");
        assert!(
            enabled(target, Level::Debug, LevelFilter::Trace),
            "{target}"
        );
    }
}

#[test]
fn the_apps_own_lines_follow_the_chosen_level() {
    assert!(enabled("murmur::actions", Level::Debug, LevelFilter::Debug));
    assert!(!enabled("murmur::actions", Level::Debug, LevelFilter::Info));
    assert!(enabled("murmur::actions", Level::Info, LevelFilter::Info));
    // A name that merely starts like a quiet dependency is not one.
    assert!(enabled("hyperion::x", Level::Debug, LevelFilter::Debug));
}

#[test]
fn a_stored_debug_level_without_debug_mode_runs_at_info() {
    use settings::LogLevel::*;
    assert_eq!(effective_log_level(false, Debug), Info);
    assert_eq!(effective_log_level(true, Debug), Debug);
    assert_eq!(effective_log_level(false, Trace), Trace);
    assert_eq!(effective_log_level(false, Warn), Warn);
    assert_eq!(effective_log_level(false, Info), Info);
}

#[test]
fn a_new_install_logs_at_info() {
    assert_eq!(
        settings::get_default_settings().log_level,
        settings::LogLevel::Info
    );
}
