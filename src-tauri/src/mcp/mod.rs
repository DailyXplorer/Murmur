//! `murmur mcp`: your meetings, read-only, for the AI apps you connect.
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
#[cfg(test)]
mod tests;
mod tools;

use std::path::{Path, PathBuf};

#[doc(hidden)]
pub use sample::seed_sample_data_dir;
pub use tools::open_read_only;

/// The app's bundle identifier, which names its data directory. A test pins
/// it to `tauri.conf.json`.
const IDENTIFIER: &str = "com.dailyxplorer.murmur";

/// Points the server at another data directory (tests and measurements).
const DATA_DIR_VARIABLE: &str = "MURMUR_MCP_DATA_DIR";

/// Where Tauri keeps the app's data on macOS.
fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DATA_DIR_VARIABLE) {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join("Library/Application Support")
            .join(IDENTIFIER),
    )
}

/// Serves until the AI app closes our stdin. The exit code of the process.
pub fn run() -> i32 {
    let Some(data_dir) = data_dir() else {
        eprintln!("murmur mcp: cannot find the home folder");
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
