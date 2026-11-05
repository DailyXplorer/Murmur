//! `murmur mcp`: your meetings and dictations, read-only, for the AI apps you
//! connect.
//!
//! An AI app (Claude Code, Codex, Claude Desktop) launches this as a child
//! process and talks to it over stdin and stdout (Model Context Protocol,
//! one JSON message per line). It starts no window, no tray, no microphone
//! and no listener. It opens the databases read-only, takes no file path
//! from the caller, and answers nothing until the user turned it on in the
//! settings (`access`). See the "Read-only MCP server" section of AGENTS.md.

mod access;
mod rpc;
mod sample;
mod setup;
#[cfg(test)]
mod tests;
mod tools;

use std::ffi::OsString;
use std::path::PathBuf;

#[doc(hidden)]
pub use sample::seed_sample_data_dir;
pub use setup::command_path;
pub use tools::open_read_only;

/// The app's bundle identifier, which names its data directory. A test pins
/// it to `tauri.conf.json`.
const IDENTIFIER: &str = "com.dailyxplorer.murmur";

/// Points the server at another data directory (tests and measurements).
const DATA_DIR_VARIABLE: &str = "MURMUR_MCP_DATA_DIR";

/// The folder Tauri keeps app data folders in on macOS.
#[cfg(target_os = "macos")]
fn data_root(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    Some(PathBuf::from(var("HOME")?).join("Library/Application Support"))
}

/// The folder Tauri keeps app data folders in on Windows: the Roaming
/// AppData folder.
#[cfg(target_os = "windows")]
fn data_root(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    Some(PathBuf::from(var("APPDATA")?))
}

/// Where the app keeps its data, read through `var` (the environment).
fn data_dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = var(DATA_DIR_VARIABLE) {
        return Some(PathBuf::from(dir));
    }
    Some(data_root(var)?.join(IDENTIFIER))
}

fn environment(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Serves until the AI app closes our stdin. The exit code of the process.
pub fn run() -> i32 {
    let Some(data_dir) = data_dir_from(environment) else {
        eprintln!("murmur mcp: cannot find the app data folder");
        return 1;
    };
    let server = rpc::Server::new(data_dir);
    match rpc::serve(&server, std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("murmur mcp: {error}");
            1
        }
    }
}
