use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::fmt;

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
    /// Put the user's previous clipboard back after a bounded paste window,
    /// unless they copied something else meanwhile. The legacy serialized
    /// name is retained for settings compatibility.
    #[default]
    DontModify,
    /// Materialize the transcription as the new clipboard value after paste.
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

/// How much AI cleanup reshapes the dictation. It applies only when cleanup
/// runs; `Normal` is the cleanup Murmur had before styles existed.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum DictationStyle {
    Formal,
    #[default]
    Normal,
    Casual,
}

/// The style of dictations into one kind of app, when it is not the general
/// `DictationStyle`. `Off` pastes them without AI cleanup.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum CategoryStyle {
    Formal,
    Normal,
    Casual,
    Off,
}

/// Which sources a meeting records. Without system audio support (before
/// macOS 14.4, or on Intel) every mode records the microphone only; the
/// stored choice is kept for when it becomes available.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeetingCaptureMode {
    #[default]
    Both,
    SystemOnly,
    MicOnly,
}

/// How long a processed meeting keeps its audio. Its transcript and summary
/// stay either way. The names are spelled out: serde and the TypeScript
/// bindings split `Days7` differently.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
pub enum MeetingAudioRetention {
    #[default]
    #[serde(rename = "keep")]
    Keep,
    #[serde(rename = "days7")]
    Days7,
    #[serde(rename = "days30")]
    Days30,
    #[serde(rename = "days90")]
    Days90,
}
