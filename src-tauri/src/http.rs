//! One HTTP client shared by every request to the ChatGPT backend, so a
//! dictation reuses the connection (and its DNS, TCP and TLS setup) an
//! earlier request or a key-press warm-up left open. The cleanup request
//! goes out on the connection the transcription just used.
//!
//! Per-request deadlines stay with the caller (`RequestBuilder::timeout`, or
//! a deadline around the whole request); only the connect timeout is a client
//! property.

use log::debug;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::codex_transcribe::CONNECT_TIMEOUT;

/// Idle connections stay pooled this long, so a pause between dictations does
/// not cost a new handshake.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// The warm-up only needs the handshake; a slow answer is not worth waiting for.
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(5);
/// After a request, its connection is pooled for `POOL_IDLE_TIMEOUT`. A
/// warm-up this soon after one would find it open, and it stays open for a
/// dictation of a minute.
const WARM_FOR: Duration = Duration::from_secs(30);

struct State {
    client: Option<reqwest::Client>,
    /// When the client was last handed out for a request.
    last_request: Option<Instant>,
}

struct Pool {
    state: Mutex<State>,
}

static POOL: Pool = Pool::new();

impl Pool {
    const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                client: None,
                last_request: None,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn client(&self) -> reqwest::Result<reqwest::Client> {
        let mut state = self.state();
        let client = match &state.client {
            Some(client) => client.clone(),
            None => {
                let client = build()?;
                state.client = Some(client.clone());
                client
            }
        };
        state.last_request = Some(Instant::now());
        Ok(client)
    }

    fn forget(&self) {
        let mut state = self.state();
        state.client = None;
        state.last_request = None;
    }

    /// False when no warm-up was sent.
    fn warm_up(&self, url: &str) -> bool {
        let Some(origin) = origin_of(url) else {
            return false;
        };
        if is_warm(self.state().last_request, Instant::now()) {
            debug!("Connection warm-up to {origin} skipped: a request just used the connection");
            return false;
        }
        let Ok(client) = self.client() else {
            return false;
        };
        tauri::async_runtime::spawn(async move {
            let started = Instant::now();
            match client.head(&origin).timeout(WARM_UP_TIMEOUT).send().await {
                Ok(response) => debug!(
                    "Connection warm-up to {origin} answered {} in {:?}",
                    response.status(),
                    started.elapsed()
                ),
                Err(error) => debug!("Connection warm-up to {origin} failed: {error}"),
            }
        });
        true
    }
}

fn build() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .build()
}

/// Whether the connection of a request sent at `last_request` is still open
/// at `now`, with time left for the dictation that starts.
fn is_warm(last_request: Option<Instant>, now: Instant) -> bool {
    last_request.is_some_and(|at| now.saturating_duration_since(at) < WARM_FOR)
}

/// The shared client. Proxy settings are read when it is built, as for any
/// `reqwest` client; a failed request discards it (`forget`) so the next
/// request reads them again.
pub fn client() -> reqwest::Result<reqwest::Client> {
    POOL.client()
}

/// Drops the shared client after a transport failure: its pooled connection
/// may be dead after a sleep or a network change, and the proxy settings may
/// have changed.
pub fn forget() {
    POOL.forget();
}

/// Opens a connection to `url`'s host in the background so the request that
/// follows finds it ready. Fire and forget: it never blocks the caller, sends
/// no credentials and no user data (a bare `HEAD` of the site root, with the
/// app's user agent), and ignores every outcome, including an error status,
/// because the handshake is all it is for. Nothing is sent when a request
/// used the connection in the last `WARM_FOR`.
pub fn warm_up(url: &str) {
    POOL.warm_up(url);
}

/// `scheme://host[:port]/` of `url`.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|a| !a.is_empty())?;
    Some(format!("{scheme}://{authority}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn origin_drops_path_query_and_fragment() {
        assert_eq!(
            origin_of("https://chatgpt.com/backend-api/transcribe?x=1").as_deref(),
            Some("https://chatgpt.com/")
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080/a#b").as_deref(),
            Some("http://127.0.0.1:8080/")
        );
        assert_eq!(origin_of("not a url"), None);
        assert_eq!(origin_of("https:///path"), None);
    }

    /// One warm-up serves both: `warm_cloud_connection` sends a single one.
    #[test]
    fn transcription_and_cleanup_are_on_the_same_host() {
        assert_eq!(
            origin_of(crate::codex_transcribe::TRANSCRIBE_URL),
            origin_of(crate::cleanup::responses::RESPONSES_URL)
        );
    }

    #[test]
    fn a_connection_is_warm_for_thirty_seconds_after_a_request() {
        let sent = Instant::now();
        assert!(!is_warm(None, sent));
        assert!(is_warm(Some(sent), sent));
        assert!(is_warm(
            Some(sent),
            sent + WARM_FOR - Duration::from_millis(1)
        ));
        assert!(!is_warm(Some(sent), sent + WARM_FOR));
    }

    /// Accepts connections forever on a thread, counting them, and answers
    /// every request on a connection with an empty 200 (keep-alive). Sends
    /// each request head to the channel.
    fn keep_alive_server() -> (String, mpsc::Receiver<String>, mpsc::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let (heads_tx, heads_rx) = mpsc::channel();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        std::thread::spawn(move || {
            for peer in listener.incoming() {
                let Ok(mut peer) = peer else { return };
                accepted_tx.send(()).unwrap();
                let heads_tx = heads_tx.clone();
                std::thread::spawn(move || {
                    let mut buffer = [0_u8; 4096];
                    loop {
                        match peer.read(&mut buffer) {
                            Ok(0) | Err(_) => return,
                            Ok(read) => {
                                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                                let _ = heads_tx.send(head.to_ascii_lowercase());
                                let _ =
                                    peer.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n");
                            }
                        }
                    }
                });
            }
        });
        (endpoint, heads_rx, accepted_rx)
    }

    fn post(client: &reqwest::Client, endpoint: &str) -> reqwest::StatusCode {
        tauri::async_runtime::block_on(async {
            client
                .post(endpoint)
                .body("x")
                .send()
                .await
                .unwrap()
                .status()
        })
    }

    #[test]
    fn warm_up_sends_a_bare_head_and_the_next_request_reuses_the_connection() {
        let (endpoint, heads, accepted) = keep_alive_server();
        let pool = Pool::new();

        assert!(pool.warm_up(&format!("{endpoint}backend-api/transcribe")));
        let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(head.starts_with("head / http/1.1"), "{head}");
        assert!(!head.contains("authorization"), "{head}");
        assert!(!head.contains("cookie"), "{head}");
        assert!(head.contains("user-agent: murmur/"), "{head}");

        // Let the warm-up response return its connection to the pool.
        std::thread::sleep(Duration::from_millis(200));
        assert!(post(&pool.client().unwrap(), &endpoint).is_success());

        accepted.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            accepted.recv_timeout(Duration::from_millis(300)).is_err(),
            "the request opened a second connection instead of reusing the warm one"
        );
    }

    #[test]
    fn two_requests_in_a_row_share_one_connection() {
        let (endpoint, _heads, accepted) = keep_alive_server();
        let pool = Pool::new();

        assert!(post(&pool.client().unwrap(), &endpoint).is_success());
        assert!(post(&pool.client().unwrap(), &endpoint).is_success());

        accepted.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            accepted.recv_timeout(Duration::from_millis(300)).is_err(),
            "the second request opened a connection of its own"
        );
    }

    #[test]
    fn no_warm_up_goes_out_right_after_a_request() {
        let (endpoint, _heads, _accepted) = keep_alive_server();
        let pool = Pool::new();

        pool.client().unwrap();

        assert!(!pool.warm_up(&endpoint));
    }

    #[test]
    fn a_forgotten_client_is_warmed_up_again() {
        let (endpoint, heads, _accepted) = keep_alive_server();
        let pool = Pool::new();
        pool.client().unwrap();

        pool.forget();

        assert!(pool.warm_up(&endpoint));
        heads.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
