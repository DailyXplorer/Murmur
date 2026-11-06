//! Update checks. They run here, not in the settings window, which can stay
//! hidden for days while Murmur sits in the menu bar. A check only learns
//! whether a newer version exists. Installing stays a click on the settings
//! footer's "Update available".

mod checks;
mod schedule;

use checks::{Checks, Found};
#[cfg(target_os = "windows")]
use log::info;
use log::warn;
use schedule::{Moment, Schedule, TICK};
use serde::Serialize;
use specta::Type;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;
use tauri_specta::Event;

use crate::{settings, tray};

pub const TRAY_ITEM_ID: &str = "update_available";

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// What the last update check found.
#[derive(Serialize, Debug, Clone, PartialEq, Type, Event)]
pub struct UpdateStatus {
    /// The newer version, `None` when Murmur is up to date.
    pub available: Option<String>,
}

/// Starts the automatic checks: at launch, every four hours, and after the
/// Mac wakes, while update checks are on.
pub fn init(app: &AppHandle) {
    let handle = app.clone();
    app.manage(Checks::new(move |available| publish(&handle, available)));

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut schedule = Schedule::default();
        loop {
            let enabled = settings::get_settings(&app).update_checks_enabled;
            if schedule.due(Moment::now(), enabled) {
                let succeeded = app.state::<Checks>().check_quietly(find(&app)).await;
                schedule.checked(succeeded);
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

pub fn update_available(app: &AppHandle) -> bool {
    app.state::<Checks>().available().is_some()
}

fn publish(app: &AppHandle, available: Option<String>) {
    if let Err(error) = (UpdateStatus { available }).emit(app) {
        warn!("Failed to emit the update status: {error}");
    }
    tray::update_tray_menu(app, None);
}

async fn find(app: &AppHandle) -> Found {
    let update = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    let found = update.map(|update| update.version);
    #[cfg(target_os = "windows")]
    if let Some(version) = &found {
        info!("Update check found version {version}");
    }
    Ok(found)
}

#[tauri::command]
#[specta::specta]
pub fn get_update_status(app: AppHandle) -> UpdateStatus {
    UpdateStatus {
        available: app.state::<Checks>().available(),
    }
}

/// The footer's "Check for updates".
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateStatus, String> {
    if !settings::get_settings(&app).update_checks_enabled {
        return Err("Update checks are turned off".to_string());
    }
    let available = app.state::<Checks>().check_now(find(&app)).await?;
    Ok(UpdateStatus { available })
}

/// The updater offers a build only when the announced version is greater by
/// semver, and the beta numbers (`1.9.0-beta.N`) must climb that way.
#[cfg(test)]
mod versions {
    use semver::Version;

    fn parse(version: &str) -> Version {
        Version::parse(version).unwrap()
    }

    #[test]
    fn a_later_beta_is_newer() {
        assert!(parse("1.9.0-beta.7") > parse("1.9.0-beta.6"));
        assert!(parse("1.9.0-beta.10") > parse("1.9.0-beta.9"));
    }

    #[test]
    fn a_beta_is_older_than_its_release() {
        assert!(parse("1.9.0-beta.7") < parse("1.9.0"));
    }

    #[test]
    fn this_build_has_a_version_the_updater_can_compare() {
        parse(env!("CARGO_PKG_VERSION"));
    }
}
