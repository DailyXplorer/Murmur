//! What the user allowed AI apps to read. Read from the settings file on
//! every call, so a switch turned off takes effect without restarting the
//! AI app that launched the server.

use std::path::Path;

/// The name of the settings file inside the app data directory.
const SETTINGS_FILE: &str = "settings_store.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    /// Meetings may be read.
    pub meetings: bool,
    /// Dictations may be read too. Never true while `meetings` is false.
    pub dictations: bool,
}

impl Access {
    pub const NONE: Access = Access {
        meetings: false,
        dictations: false,
    };

    /// A missing, unreadable or odd settings file, and any value that is not
    /// exactly `true`, leaves everything off.
    pub fn read(data_dir: &Path) -> Access {
        let Ok(bytes) = std::fs::read(data_dir.join(SETTINGS_FILE)) else {
            return Access::NONE;
        };
        let Ok(store) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Access::NONE;
        };
        let on = |key: &str| store["settings"][key] == serde_json::Value::Bool(true);
        let meetings = on("mcp_enabled");
        Access {
            meetings,
            dictations: meetings && on("mcp_include_dictations"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_with(settings: &str) -> Access {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), settings).unwrap();
        Access::read(dir.path())
    }

    #[test]
    fn everything_is_off_unless_the_file_says_true() {
        assert_eq!(
            Access::read(tempfile::tempdir().unwrap().path()),
            Access::NONE
        );
        for settings in [
            "",
            "not json",
            "{}",
            r#"{"settings": {}}"#,
            r#"{"settings": {"mcp_enabled": "true"}}"#,
            r#"{"settings": {"mcp_enabled": 1}}"#,
            r#"{"settings": {"mcp_enabled": false, "mcp_include_dictations": true}}"#,
            r#"{"settings": {"mcp_include_dictations": true}}"#,
        ] {
            assert_eq!(read_with(settings), Access::NONE, "{settings}");
        }
    }

    #[test]
    fn dictations_need_their_own_switch() {
        assert_eq!(
            read_with(r#"{"settings": {"mcp_enabled": true}}"#),
            Access {
                meetings: true,
                dictations: false
            }
        );
        assert_eq!(
            read_with(r#"{"settings": {"mcp_enabled": true, "mcp_include_dictations": true}}"#),
            Access {
                meetings: true,
                dictations: true
            }
        );
    }
}
