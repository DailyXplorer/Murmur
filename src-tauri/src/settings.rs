use log::{debug, warn};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fmt;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a log level name or legacy integer from 1 through 5")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_ascii_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(
                        de::Unexpected::Unsigned(value),
                        &"1 through 5",
                    )),
                }
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<LogLevel, E> {
                let value = u64::try_from(value)
                    .map_err(|_| E::invalid_value(de::Unexpected::Signed(value), &"1 through 5"))?;
                self.visit_u64(value)
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

impl From<LogLevel> for tauri_plugin_log::LogLevel {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tauri_plugin_log::LogLevel::Trace,
            LogLevel::Debug => tauri_plugin_log::LogLevel::Debug,
            LogLevel::Info => tauri_plugin_log::LogLevel::Info,
            LogLevel::Warn => tauri_plugin_log::LogLevel::Warn,
            LogLevel::Error => tauri_plugin_log::LogLevel::Error,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayStyle {
    None,
    Minimal,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    #[default]
    CtrlV,
    Direct,
    None,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHandling {
    #[default]
    DontModify,
    CopyToClipboard,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSubmitKey {
    #[default]
    Enter,
    CtrlEnter,
    CmdEnter,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecordingRetentionPeriod {
    Never,
    PreserveLimit,
    Days3,
    Weeks2,
    Months3,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SoundTheme {
    Marimba,
    Pop,
    Custom,
}

impl SoundTheme {
    fn as_str(&self) -> &'static str {
        match self {
            SoundTheme::Marimba => "marimba",
            SoundTheme::Pop => "pop",
            SoundTheme::Custom => "custom",
        }
    }

    pub fn to_start_path(self) -> String {
        format!("resources/{}_start.wav", self.as_str())
    }

    pub fn to_stop_path(self) -> String {
        format!("resources/{}_stop.wav", self.as_str())
    }
}

/// UI appearance mode. `System` follows the OS `prefers-color-scheme`; `Light`
/// and `Dark` force one of the two palettes Murmur already ships.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// Brand accent used by the webviews and native application icons.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccentColor {
    #[default]
    Pink,
    Blue,
    Green,
    Yellow,
    Orange,
    Red,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionProvider {
    #[default]
    Codex,
    Gemini,
    Meta,
}

/// The container-level `serde(default)` (backed by the `Default` impl below)
/// guarantees every field — including ones added in the future — falls back to
/// its `get_default_settings()` value when missing from a stored settings
/// object, so a partial store can never fail the whole load (#1619).
/// Field-level defaults below take precedence where present.
/// Settings may contain user-authored and private values. Do not derive `Debug`.
#[derive(Serialize, Deserialize, Clone, Type)]
#[serde(default)]
pub struct AppSettings {
    /// Defaults to empty on partial stores; the load path merges in the
    /// default bindings for any missing keys before the settings are used.
    #[serde(default)]
    pub bindings: HashMap<String, ShortcutBinding>,
    #[serde(default = "default_push_to_talk")]
    pub push_to_talk: bool,
    #[serde(default)]
    pub audio_feedback: bool,
    #[serde(default = "default_audio_feedback_volume")]
    pub audio_feedback_volume: f32,
    #[serde(default = "default_sound_theme")]
    pub sound_theme: SoundTheme,
    #[serde(default = "default_start_hidden")]
    pub start_hidden: bool,
    #[serde(default = "default_autostart_enabled")]
    pub autostart_enabled: bool,
    #[serde(default = "default_update_checks_enabled")]
    pub update_checks_enabled: bool,
    #[serde(default = "default_show_whats_new_on_update")]
    pub show_whats_new_on_update: bool,
    /// The app version whose What's New the user has already seen.
    #[serde(default = "default_whats_new_last_seen_version")]
    pub whats_new_last_seen_version: String,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default = "default_always_on_microphone")]
    pub always_on_microphone: bool,
    #[serde(default)]
    pub selected_microphone: Option<String>,
    /// Which input channel to use on the selected microphone device.
    /// None means "average all channels" (original behavior).
    #[serde(default)]
    pub selected_channel: Option<u16>,
    #[serde(default)]
    pub clamshell_microphone: Option<String>,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_selected_language")]
    pub selected_language: String,
    #[serde(default)]
    pub transcription_provider: TranscriptionProvider,
    #[serde(default = "default_overlay_position")]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default)]
    pub custom_words: Vec<String>,
    #[serde(default = "default_word_correction_threshold")]
    pub word_correction_threshold: f64,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(default = "default_recording_retention_period")]
    pub recording_retention_period: RecordingRetentionPeriod,
    #[serde(default)]
    pub paste_method: PasteMethod,
    #[serde(default)]
    pub clipboard_handling: ClipboardHandling,
    #[serde(default = "default_auto_submit")]
    pub auto_submit: bool,
    #[serde(default)]
    pub auto_submit_key: AutoSubmitKey,
    #[serde(default)]
    pub mute_while_recording: bool,
    #[serde(default)]
    pub append_trailing_space: bool,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default = "default_theme")]
    pub theme: Theme,
    #[serde(default)]
    pub accent_color: AccentColor,
    #[serde(default)]
    pub experimental_enabled: bool,
    #[serde(default)]
    pub lazy_stream_close: bool,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_paste_delay_after_ms")]
    pub paste_delay_after_ms: u64,
    /// Debug-gated ("beta") receipt-sequenced paste: restore the clipboard only
    /// after the target app actually reads the transcript, instead of after a
    /// fixed delay. See `paste_tx`. macOS only.
    #[serde(default)]
    pub reliable_paste: bool,
    #[serde(default = "default_filler_word_removal_enabled")]
    pub filler_word_removal_enabled: bool,
    #[serde(default)]
    pub custom_filler_words: Option<Vec<String>>,
    #[serde(default)]
    pub extra_recording_buffer_ms: u64,
    #[serde(default = "default_overlay_style")]
    pub overlay_style: OverlayStyle,
}

fn default_push_to_talk() -> bool {
    true
}

fn default_always_on_microphone() -> bool {
    false
}

fn default_start_hidden() -> bool {
    false
}

fn default_autostart_enabled() -> bool {
    false
}

fn default_update_checks_enabled() -> bool {
    true
}

fn default_show_whats_new_on_update() -> bool {
    true
}

fn default_whats_new_last_seen_version() -> String {
    String::new()
}

fn default_selected_language() -> String {
    "auto".to_string()
}

fn default_overlay_position() -> OverlayPosition {
    OverlayPosition::Bottom
}

fn default_overlay_style() -> OverlayStyle {
    OverlayStyle::Minimal
}

fn default_filler_word_removal_enabled() -> bool {
    true
}

fn default_debug_mode() -> bool {
    false
}

fn default_log_level() -> LogLevel {
    LogLevel::Debug
}

fn default_word_correction_threshold() -> f64 {
    0.18
}

fn default_paste_delay_ms() -> u64 {
    60
}

fn default_paste_delay_after_ms() -> u64 {
    60
}

fn default_auto_submit() -> bool {
    false
}

fn default_history_limit() -> usize {
    5
}

fn default_recording_retention_period() -> RecordingRetentionPeriod {
    RecordingRetentionPeriod::PreserveLimit
}

fn default_audio_feedback_volume() -> f32 {
    1.0
}

fn default_sound_theme() -> SoundTheme {
    SoundTheme::Marimba
}

fn default_theme() -> Theme {
    Theme::System
}

fn default_app_language() -> String {
    tauri_plugin_os::locale()
        .map(|l| l.replace('_', "-"))
        .unwrap_or_else(|| "en".to_string())
}

fn default_show_tray_icon() -> bool {
    true
}

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";
const OBSOLETE_SETTINGS_KEYS: [&str; 2] = ["typing_tool", "external_script_path"];

pub fn get_default_settings() -> AppSettings {
    let default_shortcut = "option+space";

    let mut bindings = HashMap::new();
    bindings.insert(
        "transcribe".to_string(),
        ShortcutBinding {
            id: "transcribe".to_string(),
            name: "Transcribe".to_string(),
            description: "Converts your speech into text.".to_string(),
            default_binding: default_shortcut.to_string(),
            current_binding: default_shortcut.to_string(),
        },
    );
    bindings.insert(
        "cancel".to_string(),
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: "escape".to_string(),
        },
    );

    AppSettings {
        bindings,
        push_to_talk: default_push_to_talk(),
        audio_feedback: false,
        audio_feedback_volume: default_audio_feedback_volume(),
        sound_theme: default_sound_theme(),
        start_hidden: default_start_hidden(),
        autostart_enabled: default_autostart_enabled(),
        update_checks_enabled: default_update_checks_enabled(),
        show_whats_new_on_update: default_show_whats_new_on_update(),
        whats_new_last_seen_version: default_whats_new_last_seen_version(),
        onboarding_completed: false,
        always_on_microphone: false,
        selected_microphone: None,
        selected_channel: None,
        clamshell_microphone: None,
        selected_output_device: None,
        selected_language: "auto".to_string(),
        transcription_provider: TranscriptionProvider::default(),
        overlay_position: default_overlay_position(),
        debug_mode: false,
        log_level: default_log_level(),
        custom_words: Vec::new(),
        word_correction_threshold: default_word_correction_threshold(),
        history_limit: default_history_limit(),
        recording_retention_period: default_recording_retention_period(),
        paste_method: PasteMethod::default(),
        clipboard_handling: ClipboardHandling::default(),
        auto_submit: default_auto_submit(),
        auto_submit_key: AutoSubmitKey::default(),
        mute_while_recording: false,
        append_trailing_space: false,
        app_language: default_app_language(),
        theme: default_theme(),
        accent_color: AccentColor::default(),
        experimental_enabled: false,
        lazy_stream_close: false,
        show_tray_icon: default_show_tray_icon(),
        paste_delay_ms: default_paste_delay_ms(),
        paste_delay_after_ms: default_paste_delay_after_ms(),
        reliable_paste: false,
        filler_word_removal_enabled: default_filler_word_removal_enabled(),
        custom_filler_words: None,
        extra_recording_buffer_ms: 0,
        overlay_style: default_overlay_style(),
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        get_default_settings()
    }
}

pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    let settings = get_settings(app);
    debug!("Loaded settings");
    settings
}

fn parse_stored_settings(settings_value: &serde_json::Value) -> (AppSettings, bool) {
    let has_obsolete_keys = settings_value.as_object().is_some_and(|settings| {
        OBSOLETE_SETTINGS_KEYS
            .iter()
            .any(|key| settings.contains_key(*key))
    });

    match serde_json::from_value::<AppSettings>(settings_value.clone()) {
        Ok(settings) => (settings, has_obsolete_keys),
        Err(e) => {
            warn!("Failed to parse stored settings ({e}); salvaging valid fields");
            (salvage_settings(settings_value), true)
        }
    }
}

/// Loads persisted settings, repairing invalid fields and obsolete values
/// before returning them.
pub fn get_settings(app: &AppHandle) -> AppSettings {
    let store = app
        .store(SETTINGS_STORE_PATH)
        .expect("Failed to initialize store");

    if let Some(settings_value) = store.get("settings") {
        let (mut settings, mut updated) = parse_stored_settings(&settings_value);

        // Merge in any bindings added since this store was written.
        for (key, value) in get_default_settings().bindings {
            if let std::collections::hash_map::Entry::Vacant(entry) = settings.bindings.entry(key) {
                debug!("Adding missing binding: {}", entry.key());
                entry.insert(value);
                updated = true;
            }
        }

        if updated {
            store.set("settings", serde_json::to_value(&settings).unwrap());
        }

        settings
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    }
}

/// Rebuilds settings from a store value that failed to deserialize as a whole.
/// Every stored field that is individually valid is kept; only broken values
/// (e.g. an enum variant written by a newer or older version) fall back to
/// their default. This means one bad field can never reset the rest of the
/// user's configuration (#1619).
fn salvage_settings(stored: &serde_json::Value) -> AppSettings {
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

pub fn write_settings(app: &AppHandle, settings: AppSettings) {
    let store = app
        .store(SETTINGS_STORE_PATH)
        .expect("Failed to initialize store");

    store.set("settings", serde_json::to_value(&settings).unwrap());
}

/// Persist settings synchronously and restore the store cache if the write
/// fails. Callers that need a transactional side effect should use this rather
/// than the fire-and-forget write helper above.
pub fn write_settings_checked(app: &AppHandle, settings: AppSettings) -> Result<(), String> {
    let store = app
        .store(SETTINGS_STORE_PATH)
        .map_err(|error| format!("Failed to initialize settings store: {error}"))?;
    let next = serde_json::to_value(&settings)
        .map_err(|error| format!("Failed to serialize settings: {error}"))?;
    let previous = store.get("settings");

    store.set("settings", next);
    if let Err(write_error) = store.save() {
        match previous {
            Some(previous) => store.set("settings", previous),
            None => {
                store.delete("settings");
            }
        }

        return match store.save() {
            Ok(()) => Err(format!("Failed to persist settings: {write_error}")),
            Err(restore_error) => Err(format!(
                "Failed to persist settings: {write_error}. Failed to restore the previous settings: {restore_error}"
            )),
        };
    }

    Ok(())
}

pub fn get_bindings(app: &AppHandle) -> HashMap<String, ShortcutBinding> {
    let settings = get_settings(app);

    settings.bindings
}

pub fn get_history_limit(app: &AppHandle) -> usize {
    let settings = get_settings(app);
    settings.history_limit
}

pub fn get_recording_retention_period(app: &AppHandle) -> RecordingRetentionPeriod {
    let settings = get_settings(app);
    settings.recording_retention_period
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_settings_json() -> serde_json::Value {
        serde_json::to_value(get_default_settings()).unwrap()
    }

    /// Every field must survive a partial store: a missing key must never fail
    /// the whole-settings parse (#1619). `json!({})` is the extreme case.
    #[test]
    fn empty_store_parses_with_defaults() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({}))
            .expect("all AppSettings fields need serde defaults");
        assert!(settings.push_to_talk);
        assert!(!settings.audio_feedback);
        assert!(settings.filler_word_removal_enabled);
        assert_eq!(settings.accent_color, AccentColor::Pink);
        assert_eq!(
            settings.transcription_provider,
            TranscriptionProvider::Codex
        );
        // Bindings default to empty; the load path merges the real defaults in.
        assert!(settings.bindings.is_empty());
    }

    #[test]
    fn log_level_accepts_legacy_numbers_and_serializes_as_names() {
        let expected = [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ];
        for (legacy, level) in (1_u64..=5).zip(expected) {
            assert_eq!(
                serde_json::from_value::<LogLevel>(serde_json::json!(legacy)).unwrap(),
                level
            );
        }
        assert_eq!(serde_json::to_value(LogLevel::Trace).unwrap(), "trace");
        assert_eq!(serde_json::to_value(LogLevel::Error).unwrap(), "error");
    }

    #[test]
    fn salvage_preserves_legacy_numeric_log_level() {
        let mut stored = default_settings_json();
        stored["log_level"] = serde_json::json!(5);
        stored["sound_theme"] = serde_json::json!("theremin");

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.log_level, LogLevel::Error);
        assert_eq!(salvaged.sound_theme, default_sound_theme());
    }

    #[test]
    fn salvage_preserves_valid_fields_when_one_value_is_invalid() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert("selected_language".into(), serde_json::json!("fr"));
        map.insert("onboarding_completed".into(), serde_json::json!(true));
        // An enum variant this build doesn't know, e.g. written by a newer
        // version before a downgrade.
        map.insert("sound_theme".into(), serde_json::json!("theremin"));
        stored["bindings"]["transcribe"]["current_binding"] = serde_json::json!("f13");

        // Precondition: this is exactly the whole-store parse failure from
        // #1619 that used to reset everything to defaults.
        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.selected_language, "fr");
        assert!(salvaged.onboarding_completed);
        assert_eq!(salvaged.bindings["transcribe"].current_binding, "f13");
        assert_eq!(salvaged.sound_theme, default_sound_theme());
    }

    #[test]
    fn transcription_provider_round_trips_and_defaults_to_codex() {
        assert_eq!(
            serde_json::to_value(TranscriptionProvider::Gemini).unwrap(),
            "gemini"
        );
        assert_eq!(
            serde_json::to_value(TranscriptionProvider::Meta).unwrap(),
            "meta"
        );
        let settings: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(
            settings.transcription_provider,
            TranscriptionProvider::Codex
        );
    }

    #[test]
    fn salvage_defaults_only_an_invalid_accent_color() {
        let mut stored = default_settings_json();
        stored["accent_color"] = serde_json::json!("ultraviolet");
        stored["selected_language"] = serde_json::json!("fr");

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.accent_color, AccentColor::Pink);
        assert_eq!(salvaged.selected_language, "fr");
    }

    #[test]
    fn salvage_drops_only_wrong_typed_fields() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert("paste_delay_ms".into(), serde_json::json!("sixty"));
        map.insert("sound_theme".into(), serde_json::json!(42));
        map.insert("custom_words".into(), serde_json::json!(["Murmur"]));

        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.paste_delay_ms, default_paste_delay_ms());
        assert_eq!(salvaged.sound_theme, default_sound_theme());
        assert_eq!(salvaged.custom_words, vec!["Murmur".to_string()]);
    }

    #[test]
    fn salvage_of_poisoned_bindings_keeps_other_fields() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        // One malformed entry poisons the whole bindings map, but must not
        // take the rest of the settings down with it.
        map.insert(
            "bindings".into(),
            serde_json::json!({ "transcribe": { "id": 42 } }),
        );
        map.insert("selected_language".into(), serde_json::json!("es"));

        assert!(serde_json::from_value::<AppSettings>(stored.clone()).is_err());

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.selected_language, "es");
        let defaults = get_default_settings();
        assert_eq!(
            salvaged.bindings["transcribe"].current_binding,
            defaults.bindings["transcribe"].current_binding
        );
    }

    #[test]
    fn salvage_tolerates_unknown_keys() {
        let mut stored = default_settings_json();
        let map = stored.as_object_mut().unwrap();
        map.insert(
            "field_from_the_future".into(),
            serde_json::json!({ "nested": true }),
        );
        map.insert("selected_language".into(), serde_json::json!("de"));
        map.insert("sound_theme".into(), serde_json::json!("theremin"));

        let salvaged = salvage_settings(&stored);
        assert_eq!(salvaged.selected_language, "de");
        assert_eq!(salvaged.sound_theme, default_sound_theme());
    }

    #[test]
    fn salvage_of_non_object_store_falls_back_to_defaults() {
        for stored in [
            serde_json::json!("corrupt"),
            serde_json::json!(null),
            serde_json::json!([1, 2, 3]),
        ] {
            let salvaged = salvage_settings(&stored);
            assert_eq!(
                serde_json::to_value(&salvaged).unwrap(),
                default_settings_json()
            );
        }
    }

    #[test]
    fn default_settings_disable_auto_submit() {
        let settings = get_default_settings();
        assert!(!settings.auto_submit);
        assert_eq!(settings.auto_submit_key, AutoSubmitKey::Enter);
        assert!(settings.whats_new_last_seen_version.is_empty());
    }

    #[test]
    fn default_overlay_style_is_minimal_when_overlay_defaults_on() {
        let settings = get_default_settings();
        assert_eq!(settings.overlay_style, OverlayStyle::Minimal);
        assert_eq!(settings.paste_method, PasteMethod::CtrlV);
        assert_eq!(
            settings.bindings["transcribe"].default_binding,
            "option+space"
        );
        assert_eq!(
            settings.bindings["transcribe"].current_binding,
            "option+space"
        );
    }

    #[test]
    fn obsolete_settings_fall_back_without_resetting_other_fields() {
        for legacy_paste_method in ["shift_insert", "ctrl_shift_v", "external_script"] {
            let mut stored = default_settings_json();
            let stored = stored.as_object_mut().unwrap();
            stored.insert("selected_language".into(), serde_json::json!("de"));
            stored.insert(
                "paste_method".into(),
                serde_json::json!(legacy_paste_method),
            );
            stored.insert("typing_tool".into(), serde_json::json!("wtype"));
            stored.insert(
                "external_script_path".into(),
                serde_json::json!("/tmp/paste"),
            );

            let settings = salvage_settings(&serde_json::Value::Object(stored.clone()));
            assert_eq!(settings.selected_language, "de");
            assert_eq!(settings.paste_method, PasteMethod::CtrlV);

            let repaired = serde_json::to_value(settings).unwrap();
            assert!(repaired.get("typing_tool").is_none());
            assert!(repaired.get("external_script_path").is_none());
        }
    }

    #[test]
    fn valid_store_without_obsolete_fields_does_not_require_rewrite() {
        let (_, updated) = parse_stored_settings(&default_settings_json());
        assert!(!updated);
    }

    #[test]
    fn valid_store_with_obsolete_fields_is_marked_for_rewrite() {
        let mut stored = default_settings_json();
        let stored = stored.as_object_mut().unwrap();
        stored.insert("selected_language".into(), serde_json::json!("de"));
        stored.insert("paste_method".into(), serde_json::json!("direct"));
        stored.insert("typing_tool".into(), serde_json::json!("wtype"));
        stored.insert(
            "external_script_path".into(),
            serde_json::json!("/tmp/paste"),
        );

        let (settings, updated) = parse_stored_settings(&serde_json::Value::Object(stored.clone()));
        assert!(updated);
        assert_eq!(settings.selected_language, "de");
        assert_eq!(settings.paste_method, PasteMethod::Direct);

        let repaired = serde_json::to_value(settings).unwrap();
        assert!(repaired.get("typing_tool").is_none());
        assert!(repaired.get("external_script_path").is_none());
    }
}
