//! The async runtime Tauri runs commands and tasks on.
//!
//! Tokio's default is one worker per core (14 on an M4 Pro, 14 idle threads
//! with their stacks). Murmur's async work is network waits, timers and short
//! hops: the CPU-heavy parts (transcription, audio, hashing, archives) run on
//! their own threads or on `spawn_blocking`, which has a separate pool that
//! this cap does not touch. No non-test code blocks a worker on `block_on`.
//! Four workers keep the small machines as responsive as before (the default
//! on a 4-core machine) while saving the rest.

use std::sync::OnceLock;

const WORKERS: usize = 4;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn build(workers: usize) -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_name("murmur-async")
        .enable_all()
        .build()
}

/// Hands Tauri a runtime with a capped worker count. Must run before anything
/// touches `tauri::async_runtime`; on failure Tauri keeps its default runtime.
pub fn install() {
    match build(WORKERS) {
        Ok(runtime) => {
            let handle = RUNTIME.get_or_init(|| runtime).handle().clone();
            tauri::async_runtime::set(handle);
        }
        Err(error) => eprintln!("Could not build the capped async runtime: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_has_the_capped_workers_and_runs_blocking_work_beside_them() {
        let runtime = build(WORKERS).unwrap();
        assert_eq!(runtime.metrics().num_workers(), WORKERS);
        // Every worker stays busy; a blocking task still completes.
        let done = runtime.block_on(async {
            let busy: Vec<_> = (0..WORKERS)
                .map(|_| {
                    tokio::spawn(async {
                        std::thread::sleep(std::time::Duration::from_millis(150))
                    })
                })
                .collect();
            let blocking = tokio::task::spawn_blocking(|| 7);
            let value = blocking.await.unwrap();
            for task in busy {
                task.await.unwrap();
            }
            value
        });
        assert_eq!(done, 7);
    }
}
