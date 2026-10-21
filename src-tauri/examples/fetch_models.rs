//! Downloads this Mac's local models into a folder with the app's own
//! downloader and manifest, without the app:
//!
//!   cargo run --example fetch_models -- <models-dir>
//!
//! The meetings helper nightly runs it, then `prepare` and the analysis.

use murmur::local_models::{
    manifest, AnalysisSupport, Host, LocalModelState, LocalModelStatus, LocalModels, PrepareFuture,
    Prepared,
};
use murmur::ProcessingOperation;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    let models = LocalModels::new(
        PathBuf::from(dir),
        manifest::this_mac(),
        AnalysisSupport::Available,
        manifest::HUGGING_FACE.to_string(),
        Arc::new(Print::default()),
    );
    models.start_download();
    let state = tauri::async_runtime::block_on(models.settled());
    println!("{state:?}");
    if state != LocalModelState::Ready {
        std::process::exit(1);
    }
}
