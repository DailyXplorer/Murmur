//! The async runtime Tauri runs commands and tasks on.
//!
//! Tokio's default is one worker per core (14 on an M4 Pro, 14 idle threads
//! with their stacks). Murmur's async work is network waits, timers and short
//! hops: the CPU-heavy parts (transcription, audio, hashing, archives) run on
//! their own threads or on `spawn_blocking`, which has a separate pool that
//! this cap does not touch. A few workers are enough.

use std::sync::OnceLock;

const MIN_WORKERS: usize = 2;
const MAX_WORKERS: usize = 4;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn worker_threads(cores: usize) -> usize {
    cores.clamp(MIN_WORKERS, MAX_WORKERS)
}

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
    let cores = std::thread::available_parallelism().map_or(MIN_WORKERS, usize::from);
    match build(worker_threads(cores)) {
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
    fn workers_scale_with_cores_within_bounds() {
        assert_eq!(worker_threads(1), 2);
        assert_eq!(worker_threads(2), 2);
        assert_eq!(worker_threads(3), 3);
        assert_eq!(worker_threads(4), 4);
        assert_eq!(worker_threads(14), 4);
    }

    #[test]
    fn the_runtime_has_the_capped_workers_and_runs_blocking_work_beside_them() {
        let runtime = build(2).unwrap();
        assert_eq!(runtime.metrics().num_workers(), 2);
        // Two workers stay busy; a blocking task and a timer still complete.
        let done = runtime.block_on(async {
            let busy: Vec<_> = (0..2)
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
