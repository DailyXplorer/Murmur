use crate::engine::Engine;
use log::{debug, error, warn};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use super::*;

pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    let settings = get_settings(app);
    debug!("Loaded settings");
    settings
}

pub(super) fn parse_stored_settings(settings_value: &serde_json::Value) -> (AppSettings, bool) {
    let has_obsolete_keys = settings_value.as_object().is_some_and(|settings| {
        OBSOLETE_SETTINGS_KEYS
            .iter()
            .any(|key| settings.contains_key(*key))
    });

    let (mut settings, mut updated) =
        match serde_json::from_value::<AppSettings>(settings_value.clone()) {
            Ok(settings) => (settings, has_obsolete_keys),
            Err(e) => {
                warn!("Failed to parse stored settings ({e}); salvaging valid fields");
                (salvage_settings(settings_value), true)
            }
        };
    // AI cleanup supersedes local filler removal: whoever turned that off
    // asked for their words untouched.
    if let Some(filler_removal) = settings_value
        .get("filler_word_removal_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        if settings_value.get("ai_cleanup_enabled").is_none() {
            settings.ai_cleanup_enabled = filler_removal;
        }
    }
    // Users from before the local engine dictated with Codex; a fresh install
    // gets its engine from onboarding.
    if settings.onboarding_completed && settings_value.get("transcription_engine").is_none() {
        settings.transcription_engine = Engine::Cloud;
        updated = true;
    }
    // Correction learning was removed, and the words it added leave the
    // dictionary; the words the user typed in stay. This runs once: the
    // store is rewritten without the obsolete `learned_words`.
    if let Some(learned) = settings_value
        .get("learned_words")
        .and_then(serde_json::Value::as_array)
    {
        let learned: HashSet<&str> = learned
            .iter()
            .filter_map(|entry| entry.get("word")?.as_str())
            .collect();
        settings
            .custom_words
            .retain(|word| !learned.contains(word.as_str()));
    }
    #[cfg(target_os = "windows")]
    {
        updated |= turn_on_window_context_once(settings_value, &mut settings);
        updated |= turn_on_update_checks_once(settings_value, &mut settings);
        updated |= turn_on_meetings_once(settings_value, &mut settings);
    }
    updated |= normalize_settings(&mut settings);
    (settings, updated)
}

fn normalize_custom_word(word: &str) -> String {
    word.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_bounded_words(words: &[String]) -> Vec<String> {
    let mut normalized_words = Vec::with_capacity(words.len().min(MAX_CUSTOM_WORDS));
    let mut seen = HashSet::with_capacity(words.len().min(MAX_CUSTOM_WORDS));
    for word in words.iter().take(MAX_CUSTOM_WORDS) {
        let normalized = normalize_custom_word(word);
        if normalized.is_empty()
            || normalized.chars().count() > MAX_CUSTOM_WORD_CHARS
            || !seen.insert(normalized.clone())
        {
            continue;
        }
        normalized_words.push(normalized);
    }
    normalized_words
}

pub fn validate_custom_words(words: &[String]) -> Result<(), String> {
    if words.len() > MAX_CUSTOM_WORDS {
        return Err(format!(
            "Custom words cannot contain more than {MAX_CUSTOM_WORDS} entries"
        ));
    }

    let mut seen = HashSet::with_capacity(words.len());
    for word in words {
        let normalized = normalize_custom_word(word);
        if normalized.is_empty() {
            return Err("Custom words cannot contain an empty entry".to_string());
        }
        if normalized != *word {
            return Err("Custom words must be normalized before saving".to_string());
        }
        if word.chars().count() > MAX_CUSTOM_WORD_CHARS {
            return Err(format!(
                "Each custom word must contain at most {MAX_CUSTOM_WORD_CHARS} characters"
            ));
        }
        if !seen.insert(word.as_str()) {
            return Err("Custom words cannot contain duplicate entries".to_string());
        }
    }
    Ok(())
}

/// Repairs persisted values before they can block the main thread or create
/// unbounded matcher work. Logs name keys only because settings are private.
pub(super) fn normalize_settings(settings: &mut AppSettings) -> bool {
    let mut updated = false;

    if settings.paste_delay_ms > MAX_PASTE_DELAY_MS {
        warn!("Resetting out-of-range setting 'paste_delay_ms'");
        settings.paste_delay_ms = default_paste_delay_ms();
        updated = true;
    }
    if settings.paste_delay_after_ms > MAX_PASTE_DELAY_MS {
        warn!("Resetting out-of-range setting 'paste_delay_after_ms'");
        settings.paste_delay_after_ms = default_paste_delay_after_ms();
        updated = true;
    }
    if settings.extra_recording_buffer_ms > MAX_EXTRA_RECORDING_BUFFER_MS {
        warn!("Resetting out-of-range setting 'extra_recording_buffer_ms'");
        settings.extra_recording_buffer_ms = 0;
        updated = true;
    }
    if settings.history_limit > MAX_HISTORY_LIMIT {
        // Clamp rather than reset: falling to the default would let the
        // retention sweep delete all but the newest few dictations.
        warn!("Clamping out-of-range setting 'history_limit'");
        settings.history_limit = MAX_HISTORY_LIMIT;
        updated = true;
    }
    if !settings.reliable_paste {
        warn!("Enabling required setting 'reliable_paste'");
        settings.reliable_paste = true;
        updated = true;
    }

    let normalized_words = normalize_bounded_words(&settings.custom_words);
    if normalized_words != settings.custom_words {
        warn!("Repairing invalid setting 'custom_words'");
        settings.custom_words = normalized_words;
        updated = true;
    }

    updated |= reconcile_bindings(&mut settings.bindings);

    #[cfg(target_os = "windows")]
    {
        updated |= restrict_to_windows(settings);
    }

    updated
}

/// Keeps exactly the bindings this version defines: adds ones introduced
/// since the store was written and drops ones a release removed, which would
/// otherwise be registered as dead global hotkeys that swallow their keys.
/// A binding the hotkey parser rejects (saved by an older version) can never
/// register, so it goes back to its default.
fn reconcile_bindings(bindings: &mut HashMap<String, ShortcutBinding>) -> bool {
    let defaults = get_default_settings().bindings;
    let before = bindings.len();
    bindings.retain(|id, _| {
        let known = defaults.contains_key(id);
        if !known {
            warn!("Removing obsolete binding '{id}'");
        }
        known
    });
    let mut updated = bindings.len() != before;
    for (id, default) in defaults {
        match bindings.entry(id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                debug!("Adding missing binding: {}", entry.key());
                entry.insert(default);
                updated = true;
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let current = &entry.get().current_binding;
                let unbound = current.is_empty() && default.default_binding.is_empty();
                if !unbound && crate::shortcut::tauri_impl::validate_binding(current).is_err() {
                    warn!(
                        "Resetting unsupported shortcut for binding '{}'",
                        entry.key()
                    );
                    entry.get_mut().current_binding = default.current_binding;
                    updated = true;
                }
            }
        }
    }
    updated
}

/// Loads persisted settings, repairing invalid fields and obsolete values
/// before returning them.
pub fn get_settings(app: &AppHandle) -> AppSettings {
    SETTINGS_TRANSACTIONS.read(|| read_settings(app))
}

// Only called while SETTINGS_TRANSACTIONS is held. Loading can itself write
// a repaired value; it must not race with a mutation.
pub(super) fn read_settings(app: &AppHandle) -> AppSettings {
    read_from(&settings_file(app).expect("Failed to initialize store"))
}

pub(super) fn read_from(file: &file::SettingsFile) -> AppSettings {
    let Some(settings_value) = file.get() else {
        let default_settings = get_default_settings();
        file.store(serde_json::to_value(&default_settings).unwrap());
        return default_settings;
    };
    let (settings, updated) = parse_stored_settings(&settings_value);
    if updated {
        if serde_json::from_value::<AppSettings>(settings_value).is_err() {
            match file.preserve_unreadable() {
                Ok(Some(copy)) => warn!(
                    "Kept the settings file as {} before dropping what this version cannot read",
                    copy.file_name().unwrap_or_default().to_string_lossy()
                ),
                Ok(None) => {}
                Err(error) => error!("Cannot keep a copy of the settings file: {error}"),
            }
        }
        file.store(serde_json::to_value(&settings).unwrap());
    }
    settings
}

/// The settings file of this app, read on first use.
pub(super) fn settings_file(
    app: &AppHandle,
) -> Result<tauri::State<'_, file::SettingsFile>, String> {
    if let Some(file) = app.try_state::<file::SettingsFile>() {
        return Ok(file);
    }
    app.manage(file::SettingsFile::open(settings_path(app)?));
    Ok(app.state::<file::SettingsFile>())
}

/// Where the store plugin of earlier versions kept it.
pub(super) fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(SETTINGS_STORE_PATH))
        .map_err(|error| format!("Cannot locate the settings file: {error}"))
}

/// Returns once the changes saved in the background are on disk. The process
/// is about to end, which would cut a pending write off.
pub fn flush_pending_save(app: &AppHandle) {
    if let Some(file) = app.try_state::<file::SettingsFile>() {
        file.flush();
    }
}
