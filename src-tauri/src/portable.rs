//! Portable mode support for Murmur.
//!
//! When a file named `portable` exists next to the executable, all user data
//! (settings, recordings, database, logs) is stored in a `Data/`
//! directory alongside the executable instead of `%APPDATA%`.

use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::Manager;

static PORTABLE_DATA_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
const CURRENT_PORTABLE_MARKER: &str = "Murmur Portable Mode";
// FNV-1a fingerprint of the exact pre-1.0 marker. Keeping only the fingerprint
// preserves portable upgrades without embedding the former product name in the
// source or compiled application.
const PRE_V1_PORTABLE_MARKER_LEN: usize = 19;
const PRE_V1_PORTABLE_MARKER_FINGERPRINT: u64 = 0x8eeb_2b9a_bf44_5609;

/// Detect portable mode by looking for a `portable` marker file next to the exe.
/// Must be called once at startup before Tauri initializes.
pub fn init() {
    PORTABLE_DATA_DIR.get_or_init(|| {
        let exe_path = std::env::current_exe().ok()?;
        let exe_dir = exe_path.parent()?;

        let marker_path = exe_dir.join("portable");
        let data_dir = exe_dir.join("Data");

        let is_portable = if is_valid_portable_marker(&marker_path) {
            true
        } else if should_upgrade_pre_v1_marker(&marker_path, &data_dir) {
            eprintln!("[portable] upgrading pre-1.0 marker");
            if let Err(error) = std::fs::write(&marker_path, CURRENT_PORTABLE_MARKER) {
                eprintln!("[portable] could not rewrite marker: {error}");
            }
            true
        } else {
            false
        };

        if is_portable {
            if !data_dir.exists() {
                std::fs::create_dir_all(&data_dir).ok()?;
            }
            eprintln!("[portable] data dir: {}", data_dir.display());
            Some(data_dir)
        } else {
            None
        }
    });
}

/// Returns `true` if running in portable mode.
pub fn is_portable() -> bool {
    PORTABLE_DATA_DIR.get().and_then(|v| v.as_ref()).is_some()
}

/// Get the portable data dir (if active). Does not require an AppHandle.
/// Returns `None` when not in portable mode.
pub fn data_dir() -> Option<&'static PathBuf> {
    PORTABLE_DATA_DIR.get().and_then(|v| v.as_ref())
}

/// Portable-aware replacement for `app.path().app_data_dir()`.
pub fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, tauri::Error> {
    if let Some(dir) = data_dir() {
        Ok(dir.clone())
    } else {
        app.path().app_data_dir()
    }
}

/// Portable-aware replacement for `app.path().app_log_dir()`.
pub fn app_log_dir(app: &tauri::AppHandle) -> Result<PathBuf, tauri::Error> {
    if let Some(dir) = data_dir() {
        Ok(dir.join("logs"))
    } else {
        app.path().app_log_dir()
    }
}

/// Resolve a relative path against the app data directory (portable-aware).
/// Replaces `app.path().resolve(path, BaseDirectory::AppData)`.
pub fn resolve_app_data(app: &tauri::AppHandle, relative: &str) -> Result<PathBuf, tauri::Error> {
    Ok(app_data_dir(app)?.join(relative))
}

/// Get the path to use with `tauri-plugin-store`.
/// Returns an absolute path in portable mode (so the store plugin writes to
/// the portable Data dir) or the original relative path otherwise.
pub fn store_path(relative: &str) -> PathBuf {
    if let Some(dir) = data_dir() {
        dir.join(relative)
    } else {
        PathBuf::from(relative)
    }
}

/// Check if a marker file contains exactly the portable magic string.
/// Extracted for testability.
fn is_valid_portable_marker(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .map(|contents| contents.trim() == CURRENT_PORTABLE_MARKER)
        .unwrap_or(false)
}

fn should_upgrade_pre_v1_marker(marker_path: &std::path::Path, data_dir: &std::path::Path) -> bool {
    if !data_dir.is_dir() {
        return false;
    }

    std::fs::read(marker_path)
        .map(|contents| {
            let trimmed = contents
                .strip_prefix(&[0xEF, 0xBB, 0xBF])
                .unwrap_or(&contents);
            let trimmed = trim_ascii_whitespace(trimmed);
            trimmed.is_empty()
                || (trimmed.len() == PRE_V1_PORTABLE_MARKER_LEN
                    && fnv1a_64(trimmed) == PRE_V1_PORTABLE_MARKER_FINGERPRINT)
        })
        .unwrap_or(false)
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_valid_magic_string_enables_portable() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        let mut f = std::fs::File::create(&marker).unwrap();
        write!(f, "Murmur Portable Mode").unwrap();
        assert!(is_valid_portable_marker(&marker));
    }

    #[test]
    fn test_empty_file_does_not_enable_portable() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        std::fs::File::create(&marker).unwrap();
        assert!(!is_valid_portable_marker(&marker));
    }

    #[test]
    fn test_wrong_content_does_not_enable_portable() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        let mut f = std::fs::File::create(&marker).unwrap();
        write!(f, "some other content").unwrap();
        assert!(!is_valid_portable_marker(&marker));
    }

    #[test]
    fn test_suffixed_magic_string_does_not_enable_portable() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        let mut f = std::fs::File::create(&marker).unwrap();
        write!(f, "Murmur Portable Mode-extra").unwrap();
        assert!(!is_valid_portable_marker(&marker));
    }

    #[test]
    fn test_pre_v1_marker_with_data_directory_is_upgraded() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("Data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let marker = dir.path().join("portable");
        let pre_v1_marker = [
            237, 196, 203, 193, 220, 133, 245, 202, 215, 209, 196, 199, 201, 192, 133, 232, 202,
            193, 192,
        ]
        .map(|byte| std::hint::black_box(byte) ^ 0xa5);
        std::fs::write(&marker, pre_v1_marker).unwrap();
        assert!(should_upgrade_pre_v1_marker(&marker, &data_dir));
    }

    #[test]
    fn test_empty_pre_v1_marker_requires_existing_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        std::fs::File::create(&marker).unwrap();
        assert!(!should_upgrade_pre_v1_marker(
            &marker,
            &dir.path().join("Data")
        ));
        std::fs::create_dir(dir.path().join("Data")).unwrap();
        assert!(should_upgrade_pre_v1_marker(
            &marker,
            &dir.path().join("Data")
        ));
    }

    #[test]
    fn test_missing_file_does_not_enable_portable() {
        let path = std::path::Path::new("/nonexistent/portable");
        assert!(!is_valid_portable_marker(path));
    }

    #[test]
    fn test_legacy_empty_marker_without_data_dir_does_not_enable_portable() {
        // Empty marker alone (scoop scenario) — no Data/ dir → not portable
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        std::fs::File::create(&marker).unwrap();
        assert!(!is_valid_portable_marker(&marker));
    }

    #[test]
    fn test_magic_string_with_whitespace_enables_portable() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("portable");
        let mut f = std::fs::File::create(&marker).unwrap();
        writeln!(f, "  Murmur Portable Mode").unwrap();
        assert!(is_valid_portable_marker(&marker));
    }
}
