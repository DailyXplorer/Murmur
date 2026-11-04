//! `settings_store.json`, held in memory and written through
//! `durable::write`. A change is written by a background thread, so a burst
//! of changes (a slider drag) costs the caller no disk sync and lands as its
//! latest value; a change that must report a failure waits for its write.
//! The file keeps the shape the store plugin of earlier versions wrote
//! (`{"settings": …}`, pretty-printed), so a Murmur from before this change
//! still reads it.

use crate::durable;
use log::{error, warn};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

const SETTINGS_KEY: &str = "settings";

pub(crate) struct SettingsFile {
    shared: Arc<Shared>,
}

struct Shared {
    path: PathBuf,
    /// Every top-level key of the file, `settings` among them.
    store: Mutex<Map<String, Value>>,
    /// Held from reading `store` until its bytes are on disk, so two writes
    /// never land out of order.
    disk: Mutex<()>,
    writer: Mutex<Writer>,
    writer_done: Condvar,
}

#[derive(Default)]
struct Writer {
    running: bool,
    dirty: bool,
}

impl SettingsFile {
    /// Reads `path`. A missing or unreadable file reads as empty, as the
    /// plugin did: `recover_damaged_store` has set any damage aside first.
    pub(crate) fn open(path: PathBuf) -> Self {
        remove_staged_copies(&path);
        let store = match fs::read(&path) {
            Ok(bytes) => decode(&bytes).unwrap_or_else(|error| {
                warn!("Cannot read the settings file: {error}");
                Map::new()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => {
                warn!("Cannot read the settings file: {error}");
                Map::new()
            }
        };
        Self {
            shared: Arc::new(Shared {
                path,
                store: Mutex::new(store),
                disk: Mutex::new(()),
                writer: Mutex::new(Writer::default()),
                writer_done: Condvar::new(),
            }),
        }
    }

    pub(crate) fn get(&self) -> Option<Value> {
        self.shared.store().get(SETTINGS_KEY).cloned()
    }

    /// Changes the value in memory only.
    pub(crate) fn set(&self, value: Option<Value>) {
        self.shared.set(value);
    }

    /// Changes the value and writes it in the background.
    pub(crate) fn store(&self, value: Value) {
        self.set(Some(value));
        let mut writer = self.shared.writer();
        writer.dirty = true;
        if writer.running {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || shared.write_pending());
        match spawned {
            Ok(_) => writer.running = true,
            Err(spawn_error) => {
                drop(writer);
                warn!("Cannot start the settings writer: {spawn_error}");
                self.flush();
            }
        }
    }

    /// Changes the value and writes it before returning. On failure the
    /// previous value is put back, in memory and on disk.
    pub(crate) fn store_checked(&self, value: Value) -> Result<(), String> {
        let _disk = self.shared.disk();
        super::transaction::persist_checked(
            value,
            || self.get(),
            |value| self.shared.set(value),
            || self.shared.write(),
        )
    }

    /// Waits for the worker, then makes one attempt at any still-dirty value.
    /// A failed attempt stays dirty for a later flush, without a retry loop.
    pub(crate) fn flush(&self) {
        let mut writer = self.shared.writer();
        while writer.running {
            writer = self
                .shared
                .writer_done
                .wait(writer)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if !writer.dirty {
            return;
        }
        writer.running = true;
        writer.dirty = false;
        drop(writer);

        let saved = self.shared.write_logged();
        let mut writer = self.shared.writer();
        writer.dirty |= !saved;
        writer.running = false;
        self.shared.writer_done.notify_all();
    }

    /// Keeps the file as it is on disk next to it, before a repair rewrites
    /// it without a field this version cannot read. One copy per distinct
    /// content, mode 0600 on macOS, with the folder's ACL on Windows.
    pub(crate) fn preserve_unreadable(&self) -> io::Result<Option<PathBuf>> {
        let bytes = match fs::read(&self.shared.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let digest = Sha256::digest(&bytes);
        let prefix: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        let mut name = self.shared.path.as_os_str().to_owned();
        name.push(format!(".unreadable-{prefix}"));
        let copy = PathBuf::from(name);
        Ok(durable::create(&copy, &bytes, 0o600)?.then_some(copy))
    }
}

impl Drop for SettingsFile {
    fn drop(&mut self) {
        self.flush();
    }
}

impl Shared {
    /// Writes the latest value, leaving failed work dirty for the next flush.
    fn write_pending(&self) {
        loop {
            {
                let mut writer = self.writer();
                if !writer.dirty {
                    writer.running = false;
                    self.writer_done.notify_all();
                    return;
                }
                writer.dirty = false;
            }
            if !self.write_logged() {
                let mut writer = self.writer();
                writer.dirty = true;
                writer.running = false;
                self.writer_done.notify_all();
                return;
            }
        }
    }

    fn write_logged(&self) -> bool {
        let _disk = self.disk();
        if let Err(write_error) = self.write() {
            error!("Cannot save the settings: {write_error}");
            return false;
        }
        true
    }

    /// Only called while `disk` is held.
    fn write(&self) -> io::Result<()> {
        let bytes = encode(&self.store());
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        durable::write(&self.path, &bytes)
    }

    fn set(&self, value: Option<Value>) {
        let mut store = self.store();
        match value {
            Some(value) => store.insert(SETTINGS_KEY.to_owned(), value),
            None => store.remove(SETTINGS_KEY),
        };
    }

    fn store(&self) -> MutexGuard<'_, Map<String, Value>> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn disk(&self) -> MutexGuard<'_, ()> {
        self.disk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The plugin's format: `serde_json::to_vec_pretty` of the top-level map.
fn encode(store: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec_pretty(store).expect("a JSON map serializes")
}

fn decode(bytes: &[u8]) -> serde_json::Result<Map<String, Value>> {
    serde_json::from_slice(bytes)
}

/// Remove only a staged copy whose owner is known to have exited. A second,
/// headless instance can open these settings while the first one writes.
fn remove_staged_copies(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_string_lossy();
        let Some(staged) = entry_name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".tmp"))
        else {
            continue;
        };
        let Some((pid, sequence)) = staged.split_once('.') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid != 0 && sequence.parse::<u64>().is_ok() && process_is_dead(pid) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(target_os = "windows")]
fn process_is_dead(pid: u32) -> bool {
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    // SAFETY: only query whether the process has exited. No process is changed.
    let process = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(process) => process,
        Err(error) => {
            // Access denied and other unknown states must preserve the file.
            return error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0);
        }
    };
    // SAFETY: this is a live owned handle, queried without waiting and closed
    // exactly once. A failed wait is not proof that the owner is dead.
    let exited = unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0;
    let _ = unsafe { CloseHandle(process) };
    exited
}

#[cfg(target_os = "macos")]
fn process_is_dead(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 checks existence and permission without sending a signal.
    (unsafe { libc::kill(pid, 0) }) == -1
        && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Bytes as tauri-plugin-store 2.4.5 saves a store holding one
    /// `settings` value: `serde_json::to_vec_pretty`, two-space indent, no
    /// trailing newline.
    const PLUGIN_FILE: &[u8] = b"{\n  \"settings\": {\n    \"history_limit\": 42,\n    \"onboarding_completed\": true,\n    \"selected_language\": \"fr\"\n  }\n}";

    #[test]
    fn settings_file_shape_matches_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        fs::write(&path, PLUGIN_FILE).unwrap();

        let file = SettingsFile::open(path.clone());
        let (settings, _) = super::super::parse_stored_settings(&file.get().unwrap());
        assert_eq!(settings.history_limit, 42);
        assert!(settings.onboarding_completed);
        assert_eq!(settings.selected_language, "fr");

        file.store_checked(file.get().unwrap()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), PLUGIN_FILE);
    }

    /// A slider drag stores a value per input event. While the disk is busy
    /// with an earlier write, each store returns at once, and the drag ends
    /// on disk as its last value.
    #[test]
    fn stores_during_a_slow_write_return_and_land_as_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let file = Arc::new(SettingsFile::open(path.clone()));
        let busy_disk = file.shared.disk();

        let (stored_tx, stored_rx) = mpsc::channel();
        let dragging = {
            let file = Arc::clone(&file);
            std::thread::spawn(move || {
                for step in 0..=100 {
                    file.store(serde_json::json!({ "audio_feedback_volume": step }));
                }
                stored_tx.send(()).unwrap();
            })
        };
        stored_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a store waited for the disk");
        dragging.join().unwrap();
        assert!(!path.exists());

        drop(busy_disk);
        file.flush();
        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["settings"]["audio_feedback_volume"], 100);
    }

    #[test]
    fn settings_file_keeps_other_top_level_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        fs::write(&path, br#"{"settings":{},"other":{"kept":true}}"#).unwrap();
        let file = SettingsFile::open(path.clone());

        file.store_checked(serde_json::json!({ "history_limit": 42 }))
            .unwrap();

        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["other"], serde_json::json!({ "kept": true }));
        assert_eq!(on_disk["settings"]["history_limit"], 42);
    }

    #[test]
    fn opening_settings_removes_only_its_staged_copies() {
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let mut owner = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(owner.wait().unwrap().success());
        let staged = dir
            .path()
            .join(format!("settings_store.json.{}.0.tmp", owner.id()));
        let unrelated = dir.path().join("other.json.0.0.tmp");
        fs::write(&staged, b"partial").unwrap();
        fs::write(&unrelated, b"kept").unwrap();

        let _file = SettingsFile::open(path);

        assert!(!staged.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"kept");
    }

    #[test]
    fn startup_cleanup_respects_a_live_staging_owner() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::process::{Command, Stdio};

        const CHILD: &str = "MURMUR_STAGING_OWNER_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            println!("staging owner ready");
            std::io::stdout().flush().unwrap();
            let _ = std::io::stdin().read(&mut [0]);
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let mut owner = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings::file::tests::startup_cleanup_respects_a_live_staging_owner",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(owner.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "owner did not start"
            );
            if line.contains("staging owner ready") {
                break;
            }
            line.clear();
        }

        let staged: Vec<PathBuf> = [
            "settings_store.json",
            "settings_store.json.bak",
            "settings_store.json.unreadable-0123456789ab",
        ]
        .iter()
        .map(|target| dir.path().join(format!("{target}.{}.0.tmp", owner.id())))
        .collect();
        let unrelated = dir
            .path()
            .join(format!("settings_store.json.not-ours.{}.0.tmp", owner.id()));
        fs::write(&unrelated, b"kept").unwrap();
        for copy in &staged {
            fs::write(copy, b"live writer's complete staging").unwrap();
        }
        let file = SettingsFile::open(path.clone());
        let live_copies: Vec<_> = staged.iter().map(|copy| fs::read(copy).ok()).collect();

        drop(owner.stdin.take());
        assert!(owner.wait().unwrap().success());
        for copy in &staged {
            fs::write(copy, b"abandoned staging").unwrap();
        }
        let reopened = SettingsFile::open(path);
        let dead_copies: Vec<_> = staged
            .iter()
            .filter(|copy| copy.exists())
            .cloned()
            .collect();
        drop((file, reopened));

        for (copy, bytes) in staged.iter().zip(live_copies) {
            assert_eq!(
                bytes.as_deref(),
                Some(b"live writer's complete staging".as_slice()),
                "live staging {}",
                copy.display()
            );
        }
        assert_eq!(dead_copies, Vec::<PathBuf>::new());
        assert_eq!(fs::read(unrelated).unwrap(), b"kept");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn flush_retries_a_background_save_after_a_reader_exhausts_the_budget() {
        use ::windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let file = SettingsFile::open(path.clone());
        file.store_checked(serde_json::json!({ "history_limit": 42 }))
            .unwrap();
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .open(&path)
            .unwrap();

        file.store(serde_json::json!({ "history_limit": 100 }));
        file.flush();
        let blocked: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(blocked["settings"]["history_limit"], 42);

        drop(reader);
        file.flush();

        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["settings"]["history_limit"], 100);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn flush_drains_a_change_queued_after_its_snapshot() {
        use ::windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::Instant;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let file = Arc::new(SettingsFile::open(path.clone()));
        file.store_checked(serde_json::json!({ "history_limit": 42 }))
            .unwrap();
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .open(&path)
            .unwrap();

        file.store(serde_json::json!({ "history_limit": 100 }));
        file.flush();
        let flushing = {
            let file = Arc::clone(&file);
            std::thread::spawn(move || file.flush())
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let staged_a = fs::read_dir(dir.path()).unwrap().flatten().any(|entry| {
                entry.file_name().to_string_lossy().ends_with(".tmp")
                    && fs::read(entry.path())
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                        .is_some_and(|value| value["settings"]["history_limit"] == 100)
            });
            if staged_a {
                break;
            }
            assert!(Instant::now() < deadline, "flush did not stage value A");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!flushing.is_finished(), "reader must still block flush A");

        file.store(serde_json::json!({ "history_limit": 200 }));
        drop(reader);
        flushing.join().unwrap();

        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk["settings"]["history_limit"], 200);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_read_only_settings_file_rolls_back_a_checked_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let file = SettingsFile::open(path.clone());
        let previous = serde_json::json!({ "history_limit": 42 });
        file.store_checked(previous.clone()).unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut read_only = original.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();

        let result = file.store_checked(serde_json::json!({ "history_limit": 5 }));
        let memory = file.get();
        let on_disk: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        fs::set_permissions(&path, original).unwrap();

        assert!(result.is_err());
        assert_eq!(memory, Some(previous.clone()));
        assert_eq!(on_disk["settings"], previous);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "a measurement for the pull request"]
    fn settings_write_latency() {
        let dir = tempfile::tempdir().unwrap();
        let file = SettingsFile::open(dir.path().join("settings_store.json"));
        let mut settings = serde_json::to_value(super::super::get_default_settings()).unwrap();
        let mut took = Vec::new();
        for change in 0..100 {
            settings["history_limit"] = serde_json::json!(change + 1);
            let start = std::time::Instant::now();
            file.store_checked(settings.clone()).unwrap();
            took.push(start.elapsed());
        }
        took.sort();
        eprintln!(
            "100 settings changes of {} bytes: p50 {:?}, p95 {:?}, max {:?}",
            fs::metadata(dir.path().join("settings_store.json"))
                .unwrap()
                .len(),
            took[49],
            took[94],
            took[99]
        );
    }
}
