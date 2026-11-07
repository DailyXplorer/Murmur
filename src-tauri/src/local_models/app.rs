//! The local models as the app runs them: status events to the windows,
//! the "resume at launch" setting, and the first load in the meetings
//! helper on Apple silicon.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_specta::Event;

use super::{
    manifest, Host, LocalModelState, LocalModelStatus, LocalModels, PrepareFuture, Prepared,
    SwitchedToLocal,
};
use crate::meetings::helper::{self, Ended};
use crate::settings::{get_settings, update_settings};
use crate::settings_window::{self, MainAction};
use crate::ProcessingOperation;

/// The first Core ML load takes about 15 s; this leaves room for a slow Mac.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

struct AppHost {
    app: AppHandle,
}

impl Host for AppHost {
    fn notify(&self, status: LocalModelStatus) {
        let ready = status.state == LocalModelState::Ready;
        if let Err(error) = status.emit(&self.app) {
            log::error!("Failed to emit the local model status: {error}");
        }
        if ready {
            switch_if_asked(&self.app, true);
        }
    }

    fn remember_wanted(&self, wanted: bool) {
        update_settings(&self.app, |settings| {
            settings.local_model_download_wanted = wanted;
        });
    }

    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        crate::meetings::capture::free_bytes(dir)
    }

    /// The speaker model is only for Meetings: a Windows user who never
    /// turned them on does not download it with the dictation model.
    #[cfg(target_os = "windows")]
    fn optional_wanted(&self) -> bool {
        get_settings(&self.app).meetings_enabled
    }

    fn optional_at_meeting_start(&self) -> bool {
        cfg!(target_os = "windows")
    }

    fn prepare(&self, dir: PathBuf, operation: ProcessingOperation) -> PrepareFuture {
        Box::pin(async move {
            // The CPU model loads in the app, with nothing to compile first.
            if !cfg!(target_arch = "aarch64") {
                return Prepared::Done;
            }
            let Some(program) = helper::helper_path() else {
                log::error!("The meetings helper is missing from the app");
                return Prepared::Failed;
            };
            let args: Vec<OsString> = vec![
                "prepare".into(),
                "--models-dir".into(),
                dir.into_os_string(),
                // Only the models being made ready are prepared.
                "--asr".into(),
                manifest::Asr::Current.helper_name().into(),
            ];
            match helper::run(&program, &args, &operation, PREPARE_TIMEOUT, |_| {}).await {
                Ok(Ended::Done) => Prepared::Done,
                Ok(Ended::Cancelled) => Prepared::Cancelled,
                Ok(ended) => {
                    log::warn!("Preparing the local models ended: {ended:?}");
                    Prepared::Failed
                }
                Err(error) => {
                    log::error!("Cannot run the meetings helper: {error}");
                    Prepared::Failed
                }
            }
        })
    }

    fn os_build(&self) -> Option<String> {
        // Like `prepare`: only Core ML compiles the models.
        if !cfg!(target_arch = "aarch64") {
            return None;
        }
        let build = kern_osversion();
        if build.is_none() {
            log::warn!("Cannot read the macOS build");
        }
        build
    }
}

/// `kern.osversion`, such as "24F74": it changes with every macOS update.
#[cfg(target_os = "macos")]
fn kern_osversion() -> Option<String> {
    let name = c"kern.osversion";
    let mut buffer = [0u8; 64];
    let mut length = buffer.len();
    // SAFETY: `buffer` holds `length` writable bytes, and sysctlbyname writes
    // at most that many, then sets `length` to what it wrote.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    let build = std::ffi::CStr::from_bytes_until_nul(&buffer[..length.min(buffer.len())])
        .ok()?
        .to_str()
        .ok()?;
    (!build.is_empty()).then(|| build.to_string())
}

/// Windows has no Core ML to compile for.
#[cfg(not(target_os = "macos"))]
fn kern_osversion() -> Option<String> {
    None
}

/// Moves dictation to this Mac if the user asked for it once the model is
/// ready, and tells the windows.
pub fn switch_if_asked(app: &AppHandle, model_ready: bool) {
    let switched = update_settings(app, |settings| {
        settings.switch_to_local_if_asked(model_ready)
    });
    if !switched {
        return;
    }
    log::info!("Dictation switched to the local engine");
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "transcription_engine"}),
    );
    // Kept for a settings window that is closed now, which tells it on open.
    settings_window::queue(app, MainAction::SwitchedToLocal);
    if let Err(error) = (SwitchedToLocal {}).emit(app) {
        log::error!("Failed to emit the switch to the local engine: {error}");
    }
}

/// On Apple silicon, the folder meetings have always used.
fn models_dir(app_data: &Path) -> PathBuf {
    if cfg!(target_arch = "aarch64") {
        app_data.join("meetings").join("models")
    } else {
        app_data.join("models")
    }
}

/// Registers the models as app state, before meetings, which use them.
pub fn init(app: &AppHandle) {
    let app_data = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log::error!("Local models are unavailable: {error}");
            return;
        }
    };
    #[cfg(target_os = "windows")]
    let app_data = crate::data_dirs_windows::models_parent(&app_data);
    // Intel Macs run the model on the CPU, without the helper.
    let support = if cfg!(target_arch = "aarch64") {
        helper::support()
    } else {
        helper::AnalysisSupport::Available
    };
    let models = LocalModels::new(
        models_dir(&app_data),
        manifest::this_mac(),
        support,
        manifest::HUGGING_FACE.to_string(),
        Arc::new(AppHost { app: app.clone() }),
    );
    models.resume_on_launch(get_settings(app).local_model_download_wanted);
    app.manage(models);
}
