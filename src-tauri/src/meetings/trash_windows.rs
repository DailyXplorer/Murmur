//! The Windows Recycle Bin for `trash.rs`, through `SHFileOperationW` with
//! `FOF_ALLOWUNDO`.
//!
//! The shell deletes for good, without a word, when it cannot recycle an item
//! (a folder bigger than the bin, a drive without one, "Don't move files to
//! the Recycle Bin" turned on). `FOF_WANTNUKEWARNING` makes it ask first, so
//! a permanent delete only happens when the person says yes to a question
//! that says so; a "no" comes back as an error and the caller puts the folder
//! back. Murmur never falls back to erasing on its own.
//!
//! What can be decided without the shell is in two plain functions, tested on
//! every host: the list of paths the shell reads (`recycle_list`) and how its
//! answer becomes a result (`outcome`).

use std::io;
use std::path::Path;

/// The longest path `SHFileOperationW` takes, its terminating NUL included.
/// It does not read `\\?\` paths, so a longer one is refused instead of
/// being cut.
const MAX_PATH: usize = 260;

/// `path` as the shell reads it: UTF-16, ended by two NULs. The shell treats
/// each path as a pattern and takes several paths in one string, so a
/// wildcard, a separator NUL or a relative path would delete something else
/// than asked: all are refused.
pub(super) fn recycle_list(path: &Path) -> io::Result<Vec<u16>> {
    let refuse = |reason: &str| {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            reason.to_owned(),
        ))
    };
    let Some(text) = path.to_str() else {
        return refuse("the path is not valid Unicode");
    };
    if text.contains(['\0', '*', '?']) {
        return refuse("the path has a wildcard or a NUL");
    }
    if text.starts_with("\\\\?\\") || text.starts_with("//?/") {
        return refuse("the Recycle Bin does not take extended-length paths");
    }
    if !is_absolute(text) {
        return refuse("the path is not absolute");
    }
    let mut list: Vec<u16> = text.encode_utf16().collect();
    if list.len() + 2 > MAX_PATH {
        return refuse("the path is too long for the Recycle Bin");
    }
    list.extend([0, 0]);
    Ok(list)
}

/// `C:\…`, `C:/…` or a `\\server\share` path.
fn is_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    drive || (text.starts_with("\\\\") && text.len() > 2)
}

/// What the shell answered: `code` is its return value (0 is success, the
/// others are its own `DE_*` codes), `aborted` is `fAnyOperationsAborted`
/// (set when the person answers no to the question about a permanent delete).
pub(super) fn outcome(code: i32, aborted: bool) -> io::Result<()> {
    if aborted {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the Recycle Bin move was cancelled",
        ));
    }
    if code != 0 {
        return Err(io::Error::other(format!(
            "the Recycle Bin refused the move (code {code:#x})"
        )));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub use system::SystemTrash;

#[cfg(target_os = "windows")]
mod system {
    use std::io;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
        FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW,
    };

    use super::{outcome, recycle_list};
    use crate::meetings::trash::Trash;

    /// The Recycle Bin of the person's PC.
    pub struct SystemTrash;

    /// COM on the calling thread, balanced on drop. A thread that already
    /// chose another apartment keeps it, and releases nothing.
    struct Com {
        owned: bool,
    }

    impl Com {
        fn enter() -> Self {
            // SAFETY: plain COM initialisation of this thread.
            let owned = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            Self { owned }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: balances the successful CoInitializeEx in `enter`.
                unsafe { CoUninitialize() };
            }
        }
    }

    impl Trash for SystemTrash {
        fn trash(&self, path: &Path) -> io::Result<()> {
            let list = recycle_list(path)?;
            let _com = Com::enter();
            let flags = FOF_ALLOWUNDO.0
                | FOF_NOCONFIRMATION.0
                | FOF_NOERRORUI.0
                | FOF_SILENT.0
                | FOF_WANTNUKEWARNING.0;
            let mut operation = SHFILEOPSTRUCTW {
                wFunc: FO_DELETE,
                pFrom: PCWSTR(list.as_ptr()),
                fFlags: flags as u16,
                ..Default::default()
            };
            // SAFETY: `operation` is initialised, and `list` is a live,
            // double-NUL-terminated UTF-16 string for the whole call.
            let code = unsafe { SHFileOperationW(&mut operation) };
            outcome(code, operation.fAnyOperationsAborted.as_bool())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(list: &[u16]) -> String {
        String::from_utf16(&list[..list.len() - 2]).unwrap()
    }

    #[test]
    fn a_folder_path_becomes_a_double_nul_ended_list() {
        let list = recycle_list(Path::new("C:\\Users\\me\\meetings\\2026-10-17 Q4 plan")).unwrap();
        assert_eq!(&list[list.len() - 2..], [0, 0]);
        assert_eq!(text(&list), "C:\\Users\\me\\meetings\\2026-10-17 Q4 plan");
        assert!(!list[..list.len() - 2].contains(&0));
    }

    #[test]
    fn a_share_path_is_absolute() {
        assert!(recycle_list(Path::new("\\\\server\\share\\meetings\\5")).is_ok());
    }

    #[test]
    fn a_path_the_shell_would_read_as_something_else_is_refused() {
        for bad in [
            "C:\\meetings\\*",
            "C:\\meetings\\2026?",
            "C:\\meetings\\a\0b",
            "meetings\\5",
            "\\meetings\\5",
            "\\\\?\\C:\\meetings\\5",
            "",
        ] {
            let error = recycle_list(Path::new(bad)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
    }

    #[test]
    fn a_path_longer_than_the_shell_reads_is_refused_not_cut() {
        let longest = format!("C:\\{}", "a".repeat(MAX_PATH - 2 - 3));
        assert!(recycle_list(Path::new(&longest)).is_ok());
        let too_long = format!("{longest}a");
        assert!(recycle_list(Path::new(&too_long)).is_err());
    }

    #[test]
    fn the_answer_of_the_shell_is_a_result() {
        assert!(outcome(0, false).is_ok());
        assert_eq!(
            outcome(0, true).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(outcome(0x78, false).is_err());
        assert!(outcome(0x75, true).is_err());
    }
}
