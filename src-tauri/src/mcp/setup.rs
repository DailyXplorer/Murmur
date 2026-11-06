//! The command an AI app is told to run, which the settings page shows.

use std::path::Path;

/// `exe` as written in an AI app's configuration. Windows can hand back a
/// verbatim path (`\\?\C:\...`), which no config file should carry: the prefix
/// is dropped for a drive path and turned back into `\\server\share` for a
/// network one. Any other path is kept as it is.
pub fn command_path(exe: &Path) -> String {
    let text = exe.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(drive_path) if drive_path.as_bytes().get(1) == Some(&b':') => drive_path.to_string(),
        _ => text.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(path: &str) -> String {
        command_path(Path::new(path))
    }

    #[test]
    fn a_verbatim_drive_path_loses_its_prefix() {
        assert_eq!(
            shown(r"\\?\C:\Users\Someone\AppData\Local\Murmur\murmur.exe"),
            r"C:\Users\Someone\AppData\Local\Murmur\murmur.exe"
        );
    }

    #[test]
    fn a_verbatim_network_path_becomes_a_unc_path() {
        assert_eq!(
            shown(r"\\?\UNC\server\share\Murmur\murmur.exe"),
            r"\\server\share\Murmur\murmur.exe"
        );
    }

    #[test]
    fn other_paths_are_kept() {
        for path in [
            r"C:\Program Files\Murmur\murmur.exe",
            r"\\server\share\murmur.exe",
            r"\\?\Volume{0}\murmur.exe",
            "/Applications/Murmur.app/Contents/MacOS/murmur",
        ] {
            assert_eq!(shown(path), path);
        }
    }
}
