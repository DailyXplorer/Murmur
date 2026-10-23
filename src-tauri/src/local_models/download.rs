//! Fetches one model file over HTTP into `<file>.part`, resuming a partial
//! file with `Range`, then checks its SHA-256 and renames it into place.
//! The design follows Handy's downloader: a connect timeout, a stall timeout
//! that keeps the partial file, and one clean retry on a damaged file.

use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use serde::Serialize;
use specta::Type;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::manifest::ModelFile;
use super::verify;
use crate::ProcessingOperation;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// No byte for this long ends the download; the partial file stays.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a download stopped, as the user is told.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadFailure {
    NoInternet,
    /// Unreachable, an HTTP error, or no data for `STALL_TIMEOUT`.
    ServerUnavailable,
    /// The file arrived twice with the wrong SHA-256.
    Damaged,
    NotEnoughSpace {
        needed_bytes: u64,
        free_bytes: u64,
    },
    /// The models folder could not be written.
    Disk,
    /// The models downloaded but would not load on this Mac.
    PrepareFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    Failed(DownloadFailure),
}

impl From<std::io::Error> for Stopped {
    fn from(error: std::io::Error) -> Self {
        log::error!("Cannot write a model file: {error}");
        Stopped::Failed(DownloadFailure::Disk)
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_default()
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

fn failure_of(error: &reqwest::Error) -> DownloadFailure {
    if error.is_connect() {
        DownloadFailure::NoInternet
    } else {
        DownloadFailure::ServerUnavailable
    }
}

pub struct Fetch<'a> {
    pub client: &'a reqwest::Client,
    pub operation: &'a ProcessingOperation,
    pub stall: Duration,
}

impl Fetch<'_> {
    /// Downloads `url` to `dest`, which then holds exactly `expected`.
    /// `on_bytes` gets the bytes of this file on disk so far.
    pub async fn file(
        &self,
        url: &str,
        dest: &Path,
        expected: &'static ModelFile,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), Stopped> {
        let part = part_path(dest);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        for attempt in 0..2 {
            self.transfer(url, &part, expected.size, on_bytes).await?;
            let checked = part.clone();
            let intact =
                tauri::async_runtime::spawn_blocking(move || verify::matches(&checked, expected))
                    .await
                    .map_err(|_| Stopped::Failed(DownloadFailure::Disk))??;
            if intact {
                std::fs::rename(&part, dest)?;
                return Ok(());
            }
            log::warn!(
                "{} arrived damaged (attempt {}), downloading it again",
                expected.path,
                attempt + 1
            );
            std::fs::remove_file(&part)?;
            on_bytes(0);
        }
        Err(Stopped::Failed(DownloadFailure::Damaged))
    }

    /// Brings `part` to `size` bytes, resuming what is already there.
    async fn transfer(
        &self,
        url: &str,
        part: &Path,
        size: u64,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), Stopped> {
        let mut have = match std::fs::metadata(part) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        if have > size {
            std::fs::remove_file(part)?;
            have = 0;
        }
        if have == size {
            return Ok(());
        }
        let mut request = self.client.get(url);
        if have > 0 {
            request = request.header(RANGE, format!("bytes={have}-"));
        }
        let mut response = self.within_stall(request.send()).await?;
        let resumed =
            response.status() == StatusCode::PARTIAL_CONTENT && starts_at(&response, have);
        if !resumed {
            if !response.status().is_success() {
                log::warn!("Model download refused: HTTP {}", response.status());
                return Err(Stopped::Failed(DownloadFailure::ServerUnavailable));
            }
            // The server ignored the range: start the file over.
            have = 0;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(part)?;
        on_bytes(have);
        while let Some(chunk) = self.within_stall(response.chunk()).await? {
            file.write_all(&chunk)?;
            have += chunk.len() as u64;
            on_bytes(have);
        }
        file.flush()?;
        if have < size {
            log::warn!("Model download cut off at {have} of {size} bytes");
            return Err(Stopped::Failed(DownloadFailure::ServerUnavailable));
        }
        Ok(())
    }

    async fn within_stall<T>(
        &self,
        step: impl std::future::Future<Output = reqwest::Result<T>>,
    ) -> Result<T, Stopped> {
        tokio::select! {
            result = tokio::time::timeout(self.stall, step) => match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => {
                    log::warn!("Model download failed: {error}");
                    Err(Stopped::Failed(failure_of(&error)))
                }
                Err(_) => {
                    log::warn!("Model download stalled for {} s", self.stall.as_secs());
                    Err(Stopped::Failed(DownloadFailure::ServerUnavailable))
                }
            },
            () = self.operation.cancelled() => Err(Stopped::Cancelled),
        }
    }
}

/// A 206 answer must continue exactly where the partial file ends.
fn starts_at(response: &reqwest::Response, offset: u64) -> bool {
    response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| value.split('-').next())
        .and_then(|start| start.parse::<u64>().ok())
        == Some(offset)
}

#[cfg(test)]
pub(super) mod test_server {
    //! A one-request-per-connection HTTP server on 127.0.0.1 that answers
    //! as each test scripts it.

    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Request {
        pub path: String,
        pub range_from: Option<u64>,
    }

    pub enum Reply {
        /// Serves `body`, honouring a range request.
        Serve(Vec<u8>),
        /// Serves `body` from its start whatever the range.
        IgnoreRange(Vec<u8>),
        /// Announces all of `body`, sends `cut` bytes of it, then closes.
        CutAfter(Vec<u8>, usize),
        /// Announces all of `body`, sends `cut` bytes, then sends nothing.
        HangAfter(Vec<u8>, usize),
        NotFound,
    }

    pub type Script = Arc<dyn Fn(&Request, usize) -> Reply + Send + Sync>;

    /// Serves until the runtime ends. Returns the base URL and every
    /// request received.
    pub async fn start(script: Script) -> (String, Arc<Mutex<Vec<Request>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let script = Arc::clone(&script);
                let log = Arc::clone(&log);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let range_from = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("range: bytes=")
                                .map(str::to_string)
                        })
                        .and_then(|range| range.trim_end_matches('-').parse().ok());
                    let request = Request { path, range_from };
                    let count = {
                        let mut log = log.lock().unwrap();
                        log.push(request.clone());
                        log.len()
                    };
                    reply(&mut socket, script(&request, count), request.range_from).await;
                });
            }
        });
        (base, seen)
    }

    async fn reply(socket: &mut tokio::net::TcpStream, reply: Reply, range_from: Option<u64>) {
        let (status, body, total, send, hang) = match reply {
            Reply::Serve(body) => match range_from {
                Some(from) if (from as usize) < body.len() => {
                    let total = body.len();
                    let rest = body[from as usize..].to_vec();
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{total}\r\n",
                        total - 1
                    );
                    let send = rest.len();
                    (header, rest, None, send, false)
                }
                _ => {
                    let send = body.len();
                    ("HTTP/1.1 200 OK\r\n".to_string(), body, None, send, false)
                }
            },
            Reply::IgnoreRange(body) => {
                let send = body.len();
                ("HTTP/1.1 200 OK\r\n".to_string(), body, None, send, false)
            }
            Reply::CutAfter(body, cut) => {
                let total = body.len();
                (
                    "HTTP/1.1 200 OK\r\n".to_string(),
                    body,
                    Some(total),
                    cut,
                    false,
                )
            }
            Reply::HangAfter(body, cut) => {
                let total = body.len();
                (
                    "HTTP/1.1 200 OK\r\n".to_string(),
                    body,
                    Some(total),
                    cut,
                    true,
                )
            }
            Reply::NotFound => (
                "HTTP/1.1 404 Not Found\r\n".to_string(),
                Vec::new(),
                None,
                0,
                false,
            ),
        };
        let length = total.unwrap_or(body.len());
        let head = format!("{status}Content-Length: {length}\r\nConnection: close\r\n\r\n");
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(&body[..send]).await;
        let _ = socket.flush().await;
        if hang {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{self, Reply, Request};
    use super::*;
    use crate::OperationId;
    use std::sync::Arc;

    fn body(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index * 7 % 251) as u8).collect()
    }

    fn expected_for(bytes: &[u8]) -> &'static ModelFile {
        use sha2::Digest;
        Box::leak(Box::new(ModelFile {
            path: "model.bin",
            size: bytes.len() as u64,
            sha256: Box::leak(format!("{:x}", sha2::Sha256::digest(bytes)).into_boxed_str()),
        }))
    }

    fn operation() -> ProcessingOperation {
        ProcessingOperation::new(OperationId(1))
    }

    async fn fetch(
        base: &str,
        dest: &Path,
        expected: &'static ModelFile,
        operation: &ProcessingOperation,
    ) -> Result<(), Stopped> {
        let client = client();
        let fetch = Fetch {
            client: &client,
            operation,
            stall: STALL_TIMEOUT,
        };
        fetch
            .file(&format!("{base}/model.bin"), dest, expected, &mut |_| {})
            .await
    }

    #[test]
    fn a_dropped_connection_resumes_where_the_file_stopped() {
        let content = body(10_000);
        let served = content.clone();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        tauri::async_runtime::block_on(async {
            let (base, seen) = test_server::start(Arc::new(move |_, count| {
                if count == 1 {
                    Reply::CutAfter(served.clone(), 4_000)
                } else {
                    Reply::Serve(served.clone())
                }
            }))
            .await;
            let expected = expected_for(&content);
            let first = fetch(&base, &dest, expected, &operation()).await;
            assert_eq!(
                first,
                Err(Stopped::Failed(DownloadFailure::ServerUnavailable))
            );
            assert_eq!(std::fs::metadata(part_path(&dest)).unwrap().len(), 4_000);

            fetch(&base, &dest, expected, &operation()).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), content);
            assert!(!part_path(&dest).exists());
            assert_eq!(
                seen.lock().unwrap().last(),
                Some(&Request {
                    path: "/model.bin".into(),
                    range_from: Some(4_000)
                })
            );
        });
    }

    #[test]
    fn a_server_that_ignores_the_range_restarts_the_file() {
        let content = body(5_000);
        let served = content.clone();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        // A partial file whose bytes are wrong: appending to it would
        // produce a damaged file.
        std::fs::write(part_path(&dest), vec![0xAA; 1_000]).unwrap();
        tauri::async_runtime::block_on(async {
            let (base, seen) =
                test_server::start(Arc::new(move |_, _| Reply::IgnoreRange(served.clone()))).await;
            fetch(&base, &dest, expected_for(&content), &operation())
                .await
                .unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), content);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].range_from, Some(1_000));
        });
    }

    #[test]
    fn a_damaged_file_is_fetched_again_once_then_reported() {
        let content = body(3_000);
        let mut damaged = content.clone();
        damaged[1_500] ^= 0xFF;
        let dir = tempfile::tempdir().unwrap();
        tauri::async_runtime::block_on(async {
            // Damaged once, then intact: the retry starts from zero.
            let (first, second) = (damaged.clone(), content.clone());
            let (base, seen) = test_server::start(Arc::new(move |_, count| {
                Reply::Serve(if count == 1 {
                    first.clone()
                } else {
                    second.clone()
                })
            }))
            .await;
            let dest = dir.path().join("once.bin");
            fetch(&base, &dest, expected_for(&content), &operation())
                .await
                .unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), content);
            assert_eq!(seen.lock().unwrap()[1].range_from, None);

            // Damaged every time.
            let always = damaged.clone();
            let (base, seen) =
                test_server::start(Arc::new(move |_, _| Reply::Serve(always.clone()))).await;
            let dest = dir.path().join("always.bin");
            assert_eq!(
                fetch(&base, &dest, expected_for(&content), &operation()).await,
                Err(Stopped::Failed(DownloadFailure::Damaged))
            );
            assert_eq!(seen.lock().unwrap().len(), 2);
            assert!(!dest.exists());
            assert!(!part_path(&dest).exists());
        });
    }

    #[test]
    fn cancelling_keeps_the_partial_file() {
        let content = body(8_000);
        let served = content.clone();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        tauri::async_runtime::block_on(async {
            let (base, _) = test_server::start(Arc::new(move |_, _| {
                Reply::HangAfter(served.clone(), 3_000)
            }))
            .await;
            let operation = operation();
            let client = client();
            let fetch = Fetch {
                client: &client,
                operation: &operation,
                stall: STALL_TIMEOUT,
            };
            let expected = expected_for(&content);
            let url = format!("{base}/model.bin");
            let cancel = operation.clone();
            let ended = fetch
                .file(&url, &dest, expected, &mut |bytes| {
                    if bytes == 3_000 {
                        cancel.cancel();
                    }
                })
                .await;
            assert_eq!(ended, Err(Stopped::Cancelled));
            assert_eq!(std::fs::metadata(part_path(&dest)).unwrap().len(), 3_000);
            assert!(!dest.exists());
        });
    }

    #[tokio::test]
    async fn a_stalled_server_times_out_and_keeps_the_partial_file() {
        let content = body(8_000);
        let served = content.clone();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let (base, _) = test_server::start(Arc::new(move |_, _| {
            Reply::HangAfter(served.clone(), 2_000)
        }))
        .await;
        let operation = operation();
        let client = client();
        let fetch = Fetch {
            client: &client,
            operation: &operation,
            stall: STALL_TIMEOUT,
        };
        // The clock stops once the bytes are in: macOS finishes a loopback
        // connection on its own time, which a clock paused from the start
        // would skip past.
        let mut paused_at = None;
        let ended = fetch
            .file(
                &format!("{base}/model.bin"),
                &dest,
                expected_for(&content),
                &mut |bytes| {
                    if bytes == 2_000 && paused_at.is_none() {
                        tokio::time::pause();
                        paused_at = Some(tokio::time::Instant::now());
                    }
                },
            )
            .await;
        assert_eq!(
            ended,
            Err(Stopped::Failed(DownloadFailure::ServerUnavailable))
        );
        assert!(paused_at.unwrap().elapsed() >= STALL_TIMEOUT);
        assert_eq!(std::fs::metadata(part_path(&dest)).unwrap().len(), 2_000);
    }

    #[test]
    fn an_http_error_is_a_server_failure_and_no_server_is_no_internet() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        tauri::async_runtime::block_on(async {
            let (base, _) = test_server::start(Arc::new(|_, _| Reply::NotFound)).await;
            assert_eq!(
                fetch(&base, &dest, expected_for(b"abc"), &operation()).await,
                Err(Stopped::Failed(DownloadFailure::ServerUnavailable))
            );
            // Nothing listens on the discard port.
            assert_eq!(
                fetch(
                    "http://127.0.0.1:9",
                    &dest,
                    expected_for(b"abc"),
                    &operation()
                )
                .await,
                Err(Stopped::Failed(DownloadFailure::NoInternet))
            );
        });
    }
}
