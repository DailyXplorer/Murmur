use crate::durable;
use chrono::{DateTime, Utc};
use log::{error, warn};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use super::*;

/// What replaced a damaged settings file at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRecovery {
    /// The copy kept at the last launch that could read the file.
    Restored,
    /// The defaults, except that no history is deleted until the user
    /// chooses how long to keep it.
    Reset,
}

/// Checks the settings file before anything reads it. A file that cannot
/// be read would otherwise read as a first launch and get the defaults
/// saved over it: every setting reset, and the default history limit would
/// then delete all but 5 dictations. Saves are atomic now, but a file an
/// earlier version cut short, or a damaged disk, can still hold one. A
/// damaged file is kept as `<name>.corrupt-<time>`, like a damaged
/// database, and replaced.
pub fn recover_damaged_store(app: &AppHandle) -> Option<SettingsRecovery> {
    let path = match settings_path(app) {
        Ok(path) => path,
        Err(error) => {
            warn!("{error}");
            return None;
        }
    };
    let (recovery, settings) = check_store_file(&path, Utc::now())?;
    // This launch reads the replacement even when the disk refused it.
    match settings_file(app) {
        Ok(file) => file.set(Some(settings)),
        Err(error) => error!("Cannot open the settings file: {error}"),
    }
    Some(recovery)
}

/// `None` when the file holds settings, which are then copied to
/// `<name>.bak`, or does not exist yet. A damaged file gives what replaced
/// it: that copy, or the defaults.
pub(super) fn check_store_file(
    path: &Path,
    now: DateTime<Utc>,
) -> Option<(SettingsRecovery, serde_json::Value)> {
    let backup = with_suffix(path, "bak");
    match fs::read(path) {
        Ok(bytes) if stored_settings(&bytes).is_some() => {
            if let Err(error) = durable::write(&backup, &bytes) {
                warn!("Cannot keep a copy of the settings: {error}");
            }
            return None;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        _ => {}
    }

    let (recovery, settings) = match fs::read(&backup).ok().and_then(|b| stored_settings(&b)) {
        Some(settings) => (SettingsRecovery::Restored, settings),
        None => {
            let mut settings = get_default_settings();
            settings.recording_retention_period = RecordingRetentionPeriod::Never;
            let settings = serde_json::to_value(settings).expect("settings serialize to JSON");
            (SettingsRecovery::Reset, settings)
        }
    };
    let kept_as = with_suffix(path, &format!("corrupt-{}", now.format("%Y%m%d-%H%M%S")));
    error!(
        "The settings file is damaged; keeping it as {} ({recovery:?})",
        kept_as.display()
    );
    if let Err(error) = fs::hard_link(path, &kept_as) {
        error!("Cannot keep the damaged settings file: {error}");
    }
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "settings": settings }))
        .expect("settings serialize to JSON");
    if let Err(error) = durable::write(path, &bytes) {
        error!("Cannot replace the damaged settings file: {error}");
    }
    Some((recovery, settings))
}

/// The settings object of a store file, `None` when it cannot be read.
fn stored_settings(bytes: &[u8]) -> Option<serde_json::Value> {
    let mut store: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(bytes).ok()?;
    store
        .remove("settings")
        .filter(serde_json::Value::is_object)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{suffix}"));
    PathBuf::from(name)
}

/// Rebuilds settings from a store value that failed to deserialize as a whole.
/// Every stored field that is individually valid is kept; only broken values
/// (e.g. an enum variant written by a newer or older version) fall back to
/// their default. This means one bad field can never reset the rest of the
/// user's configuration.
pub(super) fn salvage_settings(stored: &serde_json::Value) -> AppSettings {
    let Some(stored_map) = stored.as_object() else {
        warn!("Stored settings are not a JSON object; falling back to defaults");
        return get_default_settings();
    };

    let mut merged = serde_json::to_value(get_default_settings())
        .expect("default settings serialize to a JSON object");

    for (key, value) in stored_map {
        let previous = merged
            .as_object_mut()
            .expect("merged settings stay an object")
            .insert(key.clone(), value.clone());
        if serde_json::from_value::<AppSettings>(merged.clone()).is_err() {
            // Log only the key: future settings may contain sensitive values.
            warn!("Dropping invalid settings field '{key}', keeping its default");
            let map = merged
                .as_object_mut()
                .expect("merged settings stay an object");
            // The default keeps 5 dictations: an unknown period deletes
            // none, as after a damaged file.
            if key == "recording_retention_period" {
                map.insert(
                    key.clone(),
                    serde_json::to_value(RecordingRetentionPeriod::Never)
                        .expect("a retention period serializes"),
                );
                continue;
            }
            match previous {
                Some(previous) => map.insert(key.clone(), previous),
                None => map.remove(key),
            };
        }
    }

    serde_json::from_value(merged).unwrap_or_else(|e| {
        warn!("Failed to reassemble salvaged settings ({e}); falling back to defaults");
        get_default_settings()
    })
}
