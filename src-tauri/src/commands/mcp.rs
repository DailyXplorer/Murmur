use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::settings::update_settings;

/// What an AI app needs to launch the read-only server.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct McpSetup {
    /// This app's executable: the AI app runs it with the `mcp` argument.
    pub command: String,
}

#[tauri::command]
#[specta::specta]
pub fn change_mcp_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update_settings(&app, |settings| settings.mcp_enabled = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_mcp_include_dictations_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    update_settings(&app, |settings| settings.mcp_include_dictations = enabled);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_mcp_setup() -> Result<McpSetup, String> {
    let command = std::env::current_exe()
        .map_err(|error| format!("Cannot locate Murmur: {error}"))?
        .to_string_lossy()
        .into_owned();
    Ok(McpSetup { command })
}
