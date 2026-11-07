//! The standard open dialog, for importing an audio file: `NSOpenPanel` on
//! macOS, the common `IFileOpenDialog` on Windows. It offers the audio
//! formats `import_audio::decode` reads, with "All files" next to them: the
//! decoder goes by content, so a file with an odd extension may still work.

#[cfg(target_os = "macos")]
use objc2::{msg_send, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSString};
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
                Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
            },
        },
    },
};

/// Asks which audio file to import. The panel greys out what is not audio and
/// needs no labels. `None` when the user cancels, or when called off the main
/// thread.
#[cfg(target_os = "macos")]
pub fn ask_audio_path(
    extensions: &[&str],
    _audio_label: &str,
    _all_label: &str,
) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    // Extensions, not content types: the app runs on macOS 10.15, which has
    // no `allowedContentTypes` (macOS 11). The typed setter is deprecated
    // for that reason only, so the message is sent by hand.
    let types: Vec<_> = extensions
        .iter()
        .map(|extension| NSString::from_str(extension))
        .collect();
    let types = NSArray::from_retained_slice(&types);
    // SAFETY: `setAllowedFileTypes:` takes an `NSArray<NSString>` and returns
    // nothing.
    unsafe {
        let _: () = msg_send![&*panel, setAllowedFileTypes: &*types];
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let path = panel.URLs().firstObject()?.path()?;
    Some(PathBuf::from(path.to_string()))
}

#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The pattern list of the "Audio" filter: `*.wav;*.m4a;...`.
#[cfg(any(target_os = "windows", test))]
fn pattern_of(extensions: &[&str]) -> String {
    extensions
        .iter()
        .map(|extension| format!("*.{extension}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Asks which audio file to import, in a dialog owned by the window that
/// has the focus. `audio_label` names the filter of the formats; `all_label`
/// the one that shows every file. `None` when the user cancels or the dialog
/// cannot open. Call it on the thread that owns the windows.
#[cfg(target_os = "windows")]
pub fn ask_audio_path(extensions: &[&str], audio_label: &str, all_label: &str) -> Option<PathBuf> {
    let audio = wide(audio_label);
    let patterns = wide(&pattern_of(extensions));
    let all = wide(all_label);
    let everything = wide("*.*");
    let filters = [
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(audio.as_ptr()),
            pszSpec: PCWSTR(patterns.as_ptr()),
        },
        COMDLG_FILTERSPEC {
            pszName: PCWSTR(all.as_ptr()),
            pszSpec: PCWSTR(everything.as_ptr()),
        },
    ];
    // SAFETY: COM calls on the calling thread, which owns the app's windows
    // and so has COM initialised. The wide strings outlive `Show`. The path
    // string from `GetDisplayName` is copied, then freed once.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)
            .ok()?;
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetFileTypeIndex(1).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_audio_filter_lists_every_extension() {
        assert_eq!(pattern_of(&["wav", "m4a"]), "*.wav;*.m4a");
        assert_eq!(pattern_of(&[]), "");
    }
}
