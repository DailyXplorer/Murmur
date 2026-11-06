//! Downloads this Mac's local models into a folder with the app's own
//! downloader and manifest, without the app:
//!
//!   cargo run --example fetch_models -- <models-dir>
//!
//! The meetings helper nightly runs it, then `prepare` and the analysis; the
//! Windows CPU model job runs it before the analysis test.

use murmur::local_models::{
    manifest, AnalysisSupport, Host, LocalModelState, LocalModelStatus, LocalModels, PrepareFuture,
    Prepared,
};
use murmur::ProcessingOperation;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Prints the downloader's log: a failed optional fetch is only logged by
/// the app, and would otherwise end this process without a word.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

/// Prints each change of state, not every progress step.
#[derive(Default)]
struct Print {
    last: Mutex<Option<std::mem::Discriminant<LocalModelState>>>,
}

impl Host for Print {
    fn notify(&self, status: LocalModelStatus) {
        let kind = std::mem::discriminant(&status.state);
        let mut last = self.last.lock().unwrap();
        if *last != Some(kind) {
            *last = Some(kind);
            println!("{:?}", status.state);
        }
    }
    fn remember_wanted(&self, _: bool) {}
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }
    /// The workflow runs the helper's `prepare` itself.
    fn prepare(&self, _: PathBuf, _: ProcessingOperation) -> PrepareFuture {
        Box::pin(async { Prepared::Done })
    }
}

fn main() {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: fetch_models <models-dir>");
        std::process::exit(2);
    };
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let models = LocalModels::new(
        PathBuf::from(dir),
        manifest::this_mac(),
        AnalysisSupport::Available,
        manifest::HUGGING_FACE.to_string(),
        Arc::new(Print::default()),
    );
    // Also fetches the optional files of models a restored cache already
    // holds as ready, which `start_download` leaves alone.
    models.download_unless_declined();
    let state = tauri::async_runtime::block_on(models.settled());
    println!("{state:?}");
    if state != LocalModelState::Ready {
        std::process::exit(1);
    }
    // The optional models (the speaker model of Windows) come after the
    // others: the process must not end before they are in place.
    let optional = tauri::async_runtime::block_on(models.optional_fetched());
    println!("optional models in place: {optional}");
    // The Windows analysis tests need the speaker model: fail here, with the
    // reason logged above, not later as a missing file.
    if cfg!(target_os = "windows") && !optional {
        eprintln!("the speaker model was not fetched");
        std::process::exit(1);
    }
}
