//! The standard save dialog, for exporting a meeting: `NSSavePanel` on macOS,
//! the common `IFileSaveDialog` on Windows.

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::{
    ffi::{c_void, OsString},
    os::windows::ffi::OsStringExt,
};
#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::{
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::{
            Input::KeyboardAndMouse::GetActiveWindow,
            Shell::{
                FileSaveDialog, IFileSaveDialog, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT,
                FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
            },
        },
    },
};

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

/// The extension of `name` without its dot, when it is a short plain one.
#[cfg(target_os = "windows")]
fn extension_of(name: &str) -> Option<&str> {
    let (_, extension) = name.rsplit_once('.')?;
    let plain = !extension.is_empty()
        && extension.len() <= 8
        && extension.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then_some(extension)
}

#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Asks where to save a file, suggesting `default_name`, in a dialog owned by
/// the window that has the focus. Windows remembers the folder between
/// uses, adds the suggested extension to a name typed without one and asks
/// before it replaces a file. `None` when the user cancels or the dialog
/// cannot open. Call it on the thread that owns the windows.
#[cfg(target_os = "windows")]
pub fn ask_save_path(default_name: &str) -> Option<PathBuf> {
    let name = wide(default_name);
    let extension = extension_of(default_name).map(wide);
    // SAFETY: COM calls on the calling thread, which owns the app's windows
    // and so has COM initialised. `name` and `extension` outlive `Show`. The
    // path string from `GetDisplayName` is copied, then freed once.
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST)
            .ok()?;
        dialog.SetFileName(PCWSTR(name.as_ptr())).ok()?;
        if let Some(extension) = &extension {
            dialog
                .SetDefaultExtension(PCWSTR(extension.as_ptr()))
                .ok()?;
        }
        let owner = GetActiveWindow();
        // Cancelling is an error code too.
        dialog.Show((!owner.is_invalid()).then_some(owner)).ok()?;
        let chosen = dialog
            .GetResult()
            .ok()?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .ok()?;
        let path = PathBuf::from(OsString::from_wide(chosen.as_wide()));
        CoTaskMemFree(Some(chosen.0.cast_const().cast::<c_void>()));
        Some(path)
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn the_suggested_extension_is_the_last_short_plain_one() {
        assert_eq!(extension_of("2026-10-17 Notes.md"), Some("md"));
        assert_eq!(extension_of("archive.tar.gz"), Some("gz"));
        assert_eq!(extension_of("2026-10-17 Meeting"), None);
        assert_eq!(extension_of("Notes."), None);
        assert_eq!(extension_of("v1.2 plan"), None);
        assert_eq!(extension_of("notes.markdown-draft"), None);
    }

    #[test]
    fn wide_text_ends_with_a_nul() {
        assert_eq!(wide("é.md"), [0xe9, 0x2e, 0x6d, 0x64, 0]);
    }
}
