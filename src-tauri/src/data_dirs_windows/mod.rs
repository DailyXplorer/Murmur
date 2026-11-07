//! Where Murmur keeps its heavy data on Windows.
//!
//! `models`, `meetings` and `recordings` live in Local app data
//! (`%LOCALAPPDATA%\com.dailyxplorer.murmur`), which never roams. The small
//! files (`settings_store.json`, `history.db`, `meetings.db`, `insights.db`)
//! stay in the Roaming folder Tauri calls the app data directory, where
//! `murmur mcp` and the docs look for them.
//!
//! `prepare` moves the folders once per launch (see `migrate`) and records
//! where each one is used from. Until it ran, and in tests, every answer is
//! the Roaming folder, which is where the data lived before.

mod migrate;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use log::warn;
use tauri::{AppHandle, Manager};

use migrate::Layout;

static LAYOUT: OnceLock<Layout> = OnceLock::new();

/// Moves the heavy folders to Local app data, once, before anything opens
/// them. Safe to call again.
pub fn prepare(app: &AppHandle) {
    LAYOUT.get_or_init(
        || match (app.path().app_data_dir(), app.path().app_local_data_dir()) {
            (Ok(roaming), Ok(local)) => {
                let layout = migrate::resolve(&roaming, &local);
                // What the library or the history set aside in Local app
                // data expires like what sits in Roaming. Done here because
                // the library is not opened when Meetings were never used.
                let now = chrono::Utc::now();
                for dir in layout.set_aside_dirs() {
                    crate::meetings::paths::purge_set_asides(&dir, now);
                }
                layout
            }
            (roaming, local) => {
                warn!(
                    "Cannot locate the app data folders ({roaming:?}, {local:?}); \
                     keeping the heavy folders where they are"
                );
                // No real Roaming folder equals this one, so every lookup
                // answers with the Roaming default.
                Layout::in_roaming(Path::new(""))
            }
        },
    );
}

/// The layout, when it was made for this Roaming folder.
fn layout_for(roaming: &Path) -> Option<&'static Layout> {
    LAYOUT.get().filter(|layout| layout.roaming() == roaming)
}

/// The folder that holds `models`.
pub fn models_parent(roaming: &Path) -> PathBuf {
    layout_for(roaming).map_or_else(
        || roaming.to_path_buf(),
        |layout| layout.models_parent().to_path_buf(),
    )
}

/// The folder of the meetings' audio.
pub fn meetings_dir(roaming: &Path) -> PathBuf {
    layout_for(roaming).map_or_else(
        || roaming.join("meetings"),
        |layout| layout.meetings().to_path_buf(),
    )
}

/// The folder of the dictation recordings.
pub fn recordings_dir(roaming: &Path) -> PathBuf {
    layout_for(roaming).map_or_else(
        || roaming.join("recordings"),
        |layout| layout.recordings().to_path_buf(),
    )
}

/// The folders besides `roaming` where what the library or the history set
/// aside lands.
pub fn set_aside_dirs(roaming: &Path) -> Vec<PathBuf> {
    layout_for(roaming).map_or_else(Vec::new, Layout::set_aside_dirs)
}
