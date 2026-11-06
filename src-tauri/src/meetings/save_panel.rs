//! The standard macOS save dialog, for exporting a meeting.

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use std::path::PathBuf;

/// Asks where to save a file, suggesting `default_name`. `None` when the
/// user cancels, or when called off the main thread.
#[cfg(target_os = "macos")]
pub fn ask_save_path(default_name: &str) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let panel = NSSavePanel::savePanel(mtm);
    panel.setNameFieldStringValue(&NSString::from_str(default_name));
    panel.setCanCreateDirectories(true);
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let path = panel.URL()?.path()?;
    Some(PathBuf::from(path.to_string()))
}

/// Murmur offers no meetings on Windows, so there is never one to export.
#[cfg(target_os = "windows")]
pub fn ask_save_path(_default_name: &str) -> Option<PathBuf> {
    None
}
