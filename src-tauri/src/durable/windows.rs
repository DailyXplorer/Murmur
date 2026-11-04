//! Windows has no directory fsync here. Flush the staged bytes first, then
//! request a same-volume write-through rename. Never fall back to copy/delete.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
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
    let target = wide_path(target)?;
    // SAFETY: both buffers are NUL-terminated and live for the call. Staging
    // in the target's folder keeps the rename on one volume. COPY_ALLOWED is
    // deliberately absent: a copy/delete would expose a partial target.
    unsafe {
        MoveFileExW(
            PCWSTR(staged.as_ptr()),
            PCWSTR(target.as_ptr()),
            flags | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"));
    }
    wide.push(0);
    Ok(wide)
}
