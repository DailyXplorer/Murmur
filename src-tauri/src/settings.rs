mod defaults;
mod file;
mod load;
mod recovery;
mod transaction;
mod types;

use crate::engine::Engine;
use crate::insights::categories::AppCategory;
pub use defaults::get_default_settings;
pub use load::{
    flush_pending_save, get_settings, load_or_create_app_settings, validate_custom_words,
};
pub use recovery::{recover_damaged_store, SettingsRecovery};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use tauri::AppHandle;
pub use types::*;

use load::*;
use recovery::*;

/// The container-level `serde(default)` (backed by the `Default` impl below)
/// makes every field, including ones added later, fall back to its
/// `get_default_settings()` value when missing from a stored settings object,
/// so a partial store can never fail the whole load.
/// Field-level defaults below take precedence where present.
/// Settings may contain user-authored and private values. Do not derive `Debug`.
#[derive(Serialize, Deserialize, Clone, Type)]
#[serde(default)]
pub struct AppSettings {
    /// Defaults to empty on partial stores; the load path merges in the
    /// default bindings for any missing keys before the settings are used.
    #[serde(default)]
    pub bindings: HashMap<String, ShortcutBinding>,
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
    /// Windows only: `update_checks_enabled` was turned on once for a store
    /// from the betas that had no updater and kept it off. New Windows
    /// stores start with it set.
    #[serde(default)]
    pub update_checks_turned_on_once: bool,
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
    /// None means "average all channels".
    #[serde(default)]
    pub selected_channel: Option<u16>,
    #[serde(default)]
    pub clamshell_microphone: Option<String>,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_selected_language")]
    pub selected_language: String,
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
    #[serde(default = "default_leading_space")]
    pub leading_space: bool,
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
    /// Required guarded paste path. AppKit does not identify the process
    /// reading promised data, so settlement waits a bounded window. macOS only.
    #[serde(default = "default_reliable_paste")]
    pub reliable_paste: bool,
    /// AI cleanup and formatting of each transcript by the Codex session.
    #[serde(default = "default_ai_cleanup_enabled")]
    pub ai_cleanup_enabled: bool,
    #[serde(default)]
    pub dictation_style: DictationStyle,
    /// The kinds of app whose dictations do not follow `dictation_style`. A
    /// kind without an entry follows it.
    #[serde(default)]
    pub category_styles: BTreeMap<AppCategory, CategoryStyle>,
    /// Offer names visible in the frontmost window to the cleanup step.
    #[serde(default = "default_window_names_enabled")]
    pub window_names_enabled: bool,
    /// Windows only: `window_names_enabled` was turned on once for a store
    /// from the betas that hid the setting and kept it off. New Windows
    /// stores start with it set.
    #[serde(default)]
    pub window_names_turned_on_once: bool,
    /// More input gain for quiet speech, and a hint that speech was faint.
    #[serde(default)]
    pub whisper_mode: bool,
    // The least time a dictation keeps recording after the key is released
    // (Debug panel). 0 leaves the adaptive tail of `audio_toolkit::release_tail`
    // alone: 200 ms, up to 600 ms while the user is still speaking. (Not a doc
    // comment: those are copied into src/bindings.ts.)
    #[serde(default)]
    pub extra_recording_buffer_ms: u64,
    #[serde(default = "default_overlay_style")]
    pub overlay_style: OverlayStyle,
    /// Meeting recording, on by default: the Meetings page, its shortcut and
    /// its tray item exist only while this is on.
    #[serde(default = "default_meetings_enabled")]
    pub meetings_enabled: bool,
    // Windows only: `meetings_enabled` was turned on once for a store from the
    // betas that kept Meetings off. New Windows stores start with it set.
    #[serde(default)]
    pub meetings_turned_on_once: bool,
    #[serde(default)]
    pub meeting_capture_mode: MeetingCaptureMode,
    #[serde(default)]
    pub meeting_audio_retention: MeetingAudioRetention,
    /// Offers to record when a call app starts using the microphone. Only
    /// while meetings are on.
    #[serde(default = "default_meeting_call_prompt")]
    pub meeting_call_prompt: bool,
    /// Shows the meeting's text while it records, made on this Mac when the
    /// local model is ready.
    #[serde(default = "default_meeting_live_transcript")]
    pub meeting_live_transcript: bool,
    /// Which engine transcribes dictations.
    #[serde(default)]
    pub transcription_engine: Engine,
    /// The one-time card announcing local transcription was closed.
    #[serde(default)]
    pub local_stt_card_dismissed: bool,
    /// The user asked for the local model, so its download resumes at launch.
    #[serde(default)]
    pub local_model_download_wanted: bool,
    /// The user asked to dictate on this Mac as soon as the local model is
    /// ready.
    #[serde(default)]
    pub switch_to_local_when_ready: bool,
    /// AI apps the user connects (`murmur mcp`) may read meetings. Off until
    /// the user turns it on; `mcp::access` reads this key straight from the
    /// settings file.
    #[serde(default)]
    pub mcp_enabled: bool,
    /// Those apps may also read dictations. Needs `mcp_enabled`.
    #[serde(default)]
    pub mcp_include_dictations: bool,
    /// Also send dictations under 20 words to the default AI cleanup. Off:
    /// they are pasted right away.
    #[serde(default)]
    pub cleanup_short_dictations: bool,
}

impl AppSettings {
    /// Moves dictation to this Mac if the user asked for it and the model is
    /// ready. Returns whether it moved.
    pub fn switch_to_local_if_asked(&mut self, model_ready: bool) -> bool {
        let switch = self.switch_to_local_when_ready && model_ready;
        if switch {
            self.switch_to_local_when_ready = false;
            self.transcription_engine = Engine::Local;
        }
        switch
    }
}

/// On by default on both platforms.
fn default_meetings_enabled() -> bool {
    true
}

fn default_meeting_call_prompt() -> bool {
    true
}

fn default_meeting_live_transcript() -> bool {
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

fn default_ai_cleanup_enabled() -> bool {
    true
}

fn default_window_names_enabled() -> bool {
    true
}

fn default_debug_mode() -> bool {
    false
}

fn default_log_level() -> LogLevel {
    LogLevel::Info
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

fn default_leading_space() -> bool {
    true
}

fn default_reliable_paste() -> bool {
    true
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
/// `push_to_talk` chose between hold and toggle; one shortcut now does both.
/// The filler-removal keys are superseded by AI cleanup (see
/// `parse_stored_settings`). Gemini transcription and Apple Intelligence
/// cleanup were removed: Codex does both, so their choice is dropped.
/// Correction learning was removed, and its words with it (see
/// `parse_stored_settings`).
const OBSOLETE_SETTINGS_KEYS: [&str; 9] = [
    "typing_tool",
    "external_script_path",
    "push_to_talk",
    "filler_word_removal_enabled",
    "custom_filler_words",
    "transcription_provider",
    "cleanup_engine",
    "learned_words",
    // 1.10.0's per-app call rules: left on disk, an "always" rule would
    // record calls without a click again after a downgrade.
    "meeting_call_policies",
];
pub const MAX_PASTE_DELAY_MS: u64 = 500;
pub const MAX_EXTRA_RECORDING_BUFFER_MS: u64 = 1_500;
pub const MAX_CUSTOM_WORDS: usize = 200;
pub const MAX_CUSTOM_WORD_CHARS: usize = 50;
pub const MAX_HISTORY_LIMIT: usize = 1_000;

/// The recording shortcut out of the box.
#[cfg(target_os = "macos")]
pub const DEFAULT_TRANSCRIBE_BINDING: &str = "option+space";
/// Ctrl+Win held: Alt+Space opens the window menu on Windows, and Ctrl and
/// Win alone trigger nothing there.
#[cfg(target_os = "windows")]
pub const DEFAULT_TRANSCRIBE_BINDING: &str = "ctrl+super";

/// Drops what the Windows build does not offer: the clamshell microphone
/// (read through macOS APIs). Returns whether anything changed.
#[cfg(target_os = "windows")]
fn restrict_to_windows(settings: &mut AppSettings) -> bool {
    settings.clamshell_microphone.take().is_some()
}

/// The betas before Meetings came to Windows forced `meetings_enabled` off on
/// every load, so nobody there chose "off": it is turned on once, as it is by
/// default. Read from the stored value: a salvaged store takes the marker
/// from the defaults. Returns whether anything changed.
#[cfg(target_os = "windows")]
fn turn_on_meetings_once(stored: &serde_json::Value, settings: &mut AppSettings) -> bool {
    if stored.get("meetings_turned_on_once").is_some() {
        return false;
    }
    settings.meetings_turned_on_once = true;
    settings.meetings_enabled = true;
    // The shortcut the betas stored could not be used in practice (AltGr).
    if let Some(binding) = settings.bindings.get_mut(crate::meetings::BINDING_ID) {
        if binding.current_binding == crate::meetings::PREVIOUS_DEFAULT_BINDING {
            binding.current_binding = crate::meetings::DEFAULT_BINDING.to_string();
        }
        if binding.default_binding == crate::meetings::PREVIOUS_DEFAULT_BINDING {
            binding.default_binding = crate::meetings::DEFAULT_BINDING.to_string();
        }
    }
    true
}

/// The betas before window context came to Windows hid the setting and
/// stored it off, so nobody there chose "off": it is turned on once, as it
/// is by default. Read from the stored value: a salvaged store takes the
/// marker from the defaults. Returns whether anything changed.
#[cfg(target_os = "windows")]
fn turn_on_window_context_once(stored: &serde_json::Value, settings: &mut AppSettings) -> bool {
    if stored.get("window_names_turned_on_once").is_some() {
        return false;
    }
    settings.window_names_turned_on_once = true;
    settings.window_names_enabled = true;
    true
}

/// The betas before the updater came to Windows forced update checks off on
/// every load, so nobody there chose "off": it is turned on once, as it is by
/// default. Read from the stored value: a salvaged store takes the marker
/// from the defaults. Returns whether anything changed.
#[cfg(target_os = "windows")]
fn turn_on_update_checks_once(stored: &serde_json::Value, settings: &mut AppSettings) -> bool {
    if stored.get("update_checks_turned_on_once").is_some() {
        return false;
    }
    settings.update_checks_turned_on_once = true;
    settings.update_checks_enabled = true;
    true
}

static SETTINGS_TRANSACTIONS: transaction::Transactions = transaction::Transactions::new();

/// Changes only the caller's owned fields on the latest settings. The whole
/// load/change/commit is serialized, including load-time repairs. Keep `change`
/// free of settings calls, application locks, events and native side effects.
pub fn update_settings<T>(app: &AppHandle, change: impl FnOnce(&mut AppSettings) -> T) -> T {
    let result = SETTINGS_TRANSACTIONS.update(
        || Ok::<_, Infallible>(read_settings(app)),
        |settings| Ok(change(settings)),
        |settings| {
            store_settings(app, settings);
            Ok(())
        },
    );
    match result {
        Ok(value) => value,
        Err(impossible) => match impossible {},
    }
}

fn store_settings(app: &AppHandle, mut settings: AppSettings) {
    normalize_settings(&mut settings);
    let file = settings_file(app).expect("Failed to initialize store");

    let next = serde_json::to_value(&settings).expect("settings serialize to JSON");
    if file.get().as_ref() != Some(&next) {
        file.store(next);
    }
}

/// Changes owned fields and persists synchronously. Mutation errors publish
/// nothing. Saving and restoring the cache on failure share the same transaction
/// lock, so rollback cannot erase a concurrent accepted change. The callback has
/// the same no-side-effects restriction as `update_settings`.
pub fn update_settings_checked<T>(
    app: &AppHandle,
    change: impl FnOnce(&mut AppSettings) -> Result<T, String>,
) -> Result<T, String> {
    SETTINGS_TRANSACTIONS.update(
        || {
            // Report initialization failure rather than panicking on this path.
            settings_file(app).map(|file| read_from(&file))
        },
        change,
        |settings| store_settings_checked(app, settings),
    )
}

fn store_settings_checked(app: &AppHandle, mut settings: AppSettings) -> Result<(), String> {
    normalize_settings(&mut settings);
    let file = settings_file(app)?;
    let next = serde_json::to_value(&settings)
        .map_err(|error| format!("Failed to serialize settings: {error}"))?;
    file.store_checked(next)
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
mod tests;
