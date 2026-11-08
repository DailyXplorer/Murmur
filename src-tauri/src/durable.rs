//! Crash-safe file writes. A crash, force quit or power cut during a write
//! leaves the old file or the new one at `path`, never part of either.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "windows")]
mod windows;

/// Replaces the file at `path` with `bytes`. The file keeps its permissions;
/// a symlink stays a link, and the file it points to is replaced.
#[cfg(unix)]
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve_links(path)?;
    let mode = match fs::metadata(&target) {
        Ok(metadata) => Some(metadata.permissions().mode() & 0o7777),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let staged = stage(&target, bytes, mode)?;
    finish(&target, &staged, fs::rename(&staged, &target))
}

/// Replaces the file at `path` with `bytes` through a write-through rename.
/// The file takes the access rules of its folder, not a POSIX mode, and a
/// read-only target is refused. A symlink stays a link, and the file it
/// points to is replaced.
#[cfg(target_os = "windows")]
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve_links(path)?;
    let staged = stage(&target, bytes, None)?;
    finish(&target, &staged, windows::replace(&staged, &target))
}

/// Writes `bytes` to `path` only if nothing is there yet, with `mode`.
/// On Windows the new file takes its folder's access rules instead.
/// `Ok(false)` when `path` already exists, which is then left untouched.
pub fn create(path: &Path, bytes: &[u8], mode: u32) -> io::Result<bool> {
    let staged = stage(path, bytes, Some(mode))?;
    // A hard link, unlike a rename, refuses to replace an existing file.
    #[cfg(unix)]
    let linked = fs::hard_link(&staged, path);
    #[cfg(target_os = "windows")]
    let linked = windows::create(&staged, path);
    let _ = fs::remove_file(&staged);
    match linked {
        Ok(()) => sync_parent(path).map(|()| true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// A complete, synced copy of `bytes` next to `target`, under a name no
/// other writer uses. Without `mode` it gets a new file's default.
fn stage(target: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<PathBuf> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut staged_name = name.to_owned();
    staged_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let staged = target.with_file_name(staged_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(mode) = mode {
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(&staged)?;
    let result = (|| {
        #[cfg(unix)]
        if let Some(mode) = mode {
            // The umask applies at creation; the copy must match exactly.
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.write_all(bytes)?;
        file.sync_all()
    })();
    // Windows cannot remove an open file without compatible sharing flags.
    #[cfg(target_os = "windows")]
    drop(file);
    finish(target, &staged, result).map(|()| staged)
}

/// On success the directory entry is synced too; on failure the staged
/// file is removed.
fn finish(target: &Path, staged: &Path, result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => sync_parent(target),
        Err(error) => {
            let _ = fs::remove_file(staged);
            Err(error)
        }
    }
}

/// A rename is durable only once the directory holding it is synced.
#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::File::open(parent)?.sync_all()
}

/// Windows cannot open a folder to sync it through `std`; the staged file was
/// synced before the rename.
#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The file `path` finally names, following symlinks even when the last one
/// points at nothing yet.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let link = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) => parent.join(link),
                    None => link,
                };
            }
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("too many levels of symbolic links"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Two complete files that differ in every byte, so a reader can tell a
    /// whole file from a mix or a cut.
    fn versions() -> [Vec<u8>; 2] {
        [vec![b'a'; 1 << 20], vec![b'b'; 1 << 20]]
    }

    /// Replacements the reader races. A plain in-place write was caught by the
    /// reader in 20 of 20 runs from 25 rounds up; each round is a write
    /// through plus a synced rename, about 0.1 s on a Windows runner.
    const ROUNDS: usize = 200;

    #[test]
    fn durable_write_replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let [first, second] = versions();
        write(&path, &first).unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let reader = {
            let (path, done) = (path.clone(), Arc::clone(&done));
            let (first, second) = (first.clone(), second.clone());
            std::thread::spawn(move || {
                let mut reads = 0;
                while !done.load(Ordering::Relaxed) {
                    let bytes = fs::read(&path).unwrap();
                    assert!(bytes == first || bytes == second, "a partial file");
                    reads += 1;
                }
                reads
            })
        };
        for round in 0..ROUNDS {
            write(&path, if round % 2 == 0 { &second } else { &first }).unwrap();
        }
        done.store(true, Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        assert_eq!(fs::read(&path).unwrap(), first);
        assert_eq!(entries(dir.path()), ["settings_store.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn durable_write_keeps_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.json");
        let link = dir.path().join("settings_store.json");
        fs::write(&real, b"old").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink("real.json", &link).unwrap();

        write(&link, b"new").unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(entries(dir.path()), ["real.json", "settings_store.json"]);
    }

    #[test]
    fn durable_write_cleans_temp_on_error() {
        let dir = tempfile::tempdir().unwrap();
        // The rename fails: a directory stands where the file goes.
        let path = dir.path().join("settings_store.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"kept").unwrap();

        assert!(write(&path, b"new").is_err());

        assert_eq!(entries(dir.path()), ["settings_store.json"]);
        assert_eq!(fs::read(path.join("inside")).unwrap(), b"kept");
    }

    #[test]
    fn durable_write_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("réglages-設定.json");
        fs::write(&path, b"old settings").unwrap();

        write(&path, b"new settings").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new settings");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn durable_create_keeps_the_first_complete_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unreadable.json");

        assert!(create(&path, b"original", 0o600).unwrap());
        assert!(!create(&path, b"replacement", 0o600).unwrap());

        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(entries(dir.path()), ["unreadable.json"]);
    }

    #[test]
    fn durable_write_reports_a_missing_folder_without_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("settings_store.json");

        assert_eq!(
            write(&path, b"new").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(entries(dir.path()).is_empty());
    }

    const KILL_TARGET: &str = "MURMUR_DURABLE_KILL_TARGET";
    const KILL_PLAIN: &str = "MURMUR_DURABLE_KILL_PLAIN";

    fn settings_like(round: u64) -> Vec<u8> {
        let padding = "x".repeat(1 << 20);
        serde_json::to_vec_pretty(&serde_json::json!({
            "settings": { "round": round, "padding": padding }
        }))
        .unwrap()
    }

    /// The child process of `durable_write_survives_sigkill`.
    #[test]
    #[ignore = "started by durable_write_survives_sigkill"]
    fn rewrite_until_killed() {
        let Some(path) = std::env::var_os(KILL_TARGET).map(PathBuf::from) else {
            return;
        };
        let plain = std::env::var_os(KILL_PLAIN).is_some();
        for round in 1.. {
            let bytes = settings_like(round);
            if plain {
                fs::write(&path, &bytes).unwrap();
            } else {
                write(&path, &bytes).unwrap();
            }
            if round == 1 {
                println!("rewriting");
            }
        }
    }

    /// Files that are not one whole version after process kills at random
    /// points of a loop rewriting 1 MB, and the staged copies left behind.
    fn kill_rewrites(plain: bool, rounds: usize) -> (usize, usize) {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        fs::write(&path, settings_like(0)).unwrap();
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut unreadable = 0;
        for _ in 0..rounds {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "durable::tests::rewrite_until_killed",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(KILL_TARGET, &path)
                .env_remove(KILL_PLAIN)
                .stdout(Stdio::piped());
            if plain {
                command.env(KILL_PLAIN, "1");
            }
            let mut child = command.spawn().unwrap();
            let stdout = BufReader::new(child.stdout.take().unwrap());
            for line in stdout.lines() {
                if line.unwrap().contains("rewriting") {
                    break;
                }
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            std::thread::sleep(std::time::Duration::from_micros(seed % 25_000));
            child.kill().unwrap();
            child.wait().unwrap();

            let bytes = fs::read(&path).unwrap();
            let whole = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| value["settings"]["round"].as_u64())
                .is_some_and(|round| bytes == settings_like(round));
            if !whole {
                unreadable += 1;
                fs::write(&path, settings_like(0)).unwrap();
            }
        }
        let staged = entries(dir.path()).len() - 1;
        (unreadable, staged)
    }

    #[test]
    #[ignore = "about a minute; run once for the pull request"]
    fn durable_write_survives_sigkill() {
        let (durable, staged) = kill_rewrites(false, 200);
        let (plain, _) = kill_rewrites(true, 200);
        eprintln!(
            "200 kills: durable::write {durable} unreadable ({staged} staged copies left), \
             fs::write {plain} unreadable"
        );
        assert_eq!(durable, 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn durable_write_survives_process_termination() {
        assert_eq!(kill_rewrites(false, 12).0, 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn durable_write_leaves_a_read_only_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        fs::write(&path, b"old").unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut read_only = original.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();

        let result = write(&path, b"new");
        let bytes = fs::read(&path).unwrap();
        let still_read_only = fs::metadata(&path).unwrap().permissions().readonly();
        fs::set_permissions(&path, original).unwrap();

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(bytes, b"old");
        assert!(still_read_only);
        assert_eq!(entries(dir.path()), ["settings_store.json"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn durable_write_waits_for_a_short_lived_reader_to_close() {
        use ::windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        fs::write(&path, b"old").unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .open(&path)
            .unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writing = {
            let path = path.clone();
            std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                done_tx.send(write(&path, b"new")).unwrap();
            })
        };
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let while_open = done_rx.recv_timeout(Duration::from_millis(50));
        drop(reader);

        assert!(matches!(while_open, Err(mpsc::RecvTimeoutError::Timeout)));
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        writing.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(entries(dir.path()), ["settings_store.json"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn durable_write_and_create_support_paths_past_max_path() {
        use std::os::windows::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy();
        let root = if let Some(unc) = root.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            root.strip_prefix(r"\\?\").unwrap_or(&root).to_owned()
        };
        let mut directory = PathBuf::from(root);
        for _ in 0..10 {
            directory.push("long-settings-directory-component");
        }
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings_store.json");
        assert!(path.as_os_str().encode_wide().count() > 260);
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));

        write(&path, b"old").unwrap();
        write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");

        let copy = directory.join("settings_store.json.unreadable-test");
        assert!(create(&copy, b"original", 0o600).unwrap());
        assert!(!create(&copy, b"replacement", 0o600).unwrap());
        assert_eq!(fs::read(copy).unwrap(), b"original");
    }
}
