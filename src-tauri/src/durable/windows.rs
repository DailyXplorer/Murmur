//! Windows has no directory fsync here. Flush the staged bytes first, then
//! request a same-volume write-through rename. Never fall back to copy/delete.

use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
};
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS,
};

pub(super) fn replace(staged: &Path, target: &Path) -> io::Result<()> {
    move_file(staged, target, MOVEFILE_REPLACE_EXISTING)
}

pub(super) fn create(staged: &Path, target: &Path) -> io::Result<()> {
    move_file(staged, target, MOVE_FILE_FLAGS(0))
}

fn move_file(staged: &Path, target: &Path, flags: MOVE_FILE_FLAGS) -> io::Result<()> {
    let staged = wide_path(staged)?;
    let target_wide = wide_path(target)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        // SAFETY: both buffers are NUL-terminated and live for the call.
        // COPY_ALLOWED is absent: a copy/delete would expose a partial target.
        let result = unsafe {
            MoveFileExW(
                PCWSTR(staged.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                flags | MOVEFILE_WRITE_THROUGH,
            )
        };
        let Err(error) = result else {
            return Ok(());
        };
        let error = io::Error::from_raw_os_error(error.code().0 & 0xffff);
        let transient = error.raw_os_error().is_some_and(|code| {
            code == ERROR_ACCESS_DENIED.0 as i32
                || code == ERROR_SHARING_VIOLATION.0 as i32
                || code == ERROR_LOCK_VIOLATION.0 as i32
        });
        // Replacement can briefly fail during concurrent reads on Windows.
        // Retry the same atomic operation, never remove the target.
        // Read-only files and directories are not transient sharing failures.
        if !transient
            || Instant::now() >= deadline
            || fs::metadata(target)
                .is_ok_and(|metadata| metadata.is_dir() || metadata.permissions().readonly())
        {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_paths_use_the_extended_unc_namespace() {
        let path = Path::new(r"\\server\share\folder\settings_store.json");
        let expected: Vec<u16> = r"\\?\UNC\server\share\folder\settings_store.json"
            .encode_utf16()
            .chain([0])
            .collect();

        assert_eq!(wide_path(path).unwrap(), expected);
    }

    #[test]
    fn extended_paths_are_not_prefixed_twice() {
        for path in [
            r"\\?\C:\folder\settings_store.json",
            r"\\?\UNC\server\share\folder\settings_store.json",
        ] {
            let expected: Vec<u16> = path.encode_utf16().chain([0]).collect();
            assert_eq!(wide_path(Path::new(path)).unwrap(), expected);
        }
    }
}
