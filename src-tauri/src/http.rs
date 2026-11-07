//! One HTTP client per connect timeout, shared by every request to the
//! ChatGPT backend, so a dictation reuses the connection (and its DNS, TCP and
//! TLS setup) an earlier request or a key-press warm-up left open.
//!
//! Per-request deadlines stay on the request (`RequestBuilder::timeout`);
//! only the connect timeout is a client property, hence one client per value
//! (two in practice: cleanup's short one and the transcription one).

use log::debug;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// Idle connections stay pooled this long, so a pause between dictations does
/// not cost a new handshake.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// The warm-up only needs the handshake; a slow answer is not worth waiting for.
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(5);

static CLIENTS: Mutex<Vec<(Duration, reqwest::Client)>> = Mutex::new(Vec::new());

fn clients() -> std::sync::MutexGuard<'static, Vec<(Duration, reqwest::Client)>> {
    CLIENTS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn build(connect_timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .connect_timeout(connect_timeout)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .build()
}

/// The shared client for requests that must connect within `connect_timeout`.
/// Proxy settings are read when it is built, as for any `reqwest` client; a
/// failed request discards it (`forget`) so the next request reads them again.
pub fn client(connect_timeout: Duration) -> reqwest::Result<reqwest::Client> {
    let mut clients = clients();
    if let Some((_, client)) = clients
        .iter()
        .find(|(timeout, _)| *timeout == connect_timeout)
    {
        return Ok(client.clone());
    }
    let client = build(connect_timeout)?;
    clients.push((connect_timeout, client.clone()));
    Ok(client)
}

/// Drops the shared client after a transport failure (connect, reset,
/// timeout): its pooled connections may be dead after a sleep or a network
/// change, and the proxy settings may have changed.
pub fn forget(connect_timeout: Duration) {
    clients().retain(|(timeout, _)| *timeout != connect_timeout);
}

/// Opens (or refreshes) a connection to `url`'s host in the background so the
/// request that follows finds it ready. Fire and forget: it never blocks the
/// caller, sends no credentials and no user data (a bare `HEAD` of the site
/// root, with the app's user agent), and ignores every outcome, including an
/// error status, because the handshake is all it is for.
pub fn warm_up(connect_timeout: Duration, url: &str) {
    let Some(origin) = origin_of(url) else {
        return;
    };
    let Ok(client) = client(connect_timeout) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        match client.head(&origin).timeout(WARM_UP_TIMEOUT).send().await {
            Ok(response) => debug!(
                "Connection warm-up to {origin} answered {} in {:?}",
                response.status(),
                started.elapsed()
            ),
            Err(error) => debug!("Connection warm-up to {origin} failed: {error}"),
        }
    });
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

    #[test]
    fn one_client_per_connect_timeout() {
        let a = Duration::from_millis(7_001);
        let b = Duration::from_millis(7_002);
        assert_eq!(clients().iter().filter(|(t, _)| *t == a).count(), 0);
        client(a).unwrap();
        client(a).unwrap();
        client(b).unwrap();
        assert_eq!(clients().iter().filter(|(t, _)| *t == a).count(), 1);
        forget(a);
        assert_eq!(clients().iter().filter(|(t, _)| *t == a).count(), 0);
        assert_eq!(clients().iter().filter(|(t, _)| *t == b).count(), 1);
        forget(b);
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

    #[test]
    fn warm_up_sends_a_bare_head_and_the_next_request_reuses_the_connection() {
        let (endpoint, heads, accepted) = keep_alive_server();
        let timeout = Duration::from_millis(7_003);

        warm_up(timeout, &format!("{endpoint}backend-api/transcribe"));
        let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(head.starts_with("head / http/1.1"), "{head}");
        assert!(!head.contains("authorization"), "{head}");
        assert!(!head.contains("cookie"), "{head}");
        assert!(head.contains("user-agent: murmur/"), "{head}");

        // Let the warm-up response return its connection to the pool.
        std::thread::sleep(Duration::from_millis(200));
        let client = client(timeout).unwrap();
        let status = tauri::async_runtime::block_on(async {
            client
                .post(&endpoint)
                .body("x")
                .send()
                .await
                .unwrap()
                .status()
        });
        assert!(status.is_success());

        accepted.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            accepted.recv_timeout(Duration::from_millis(300)).is_err(),
            "the request opened a second connection instead of reusing the warm one"
        );
        forget(timeout);
    }
}
