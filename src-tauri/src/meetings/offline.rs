//! Waiting for the connection after Codex could not be reached. The queue
//! holds the meetings not asked for, checks the connection, and lets them
//! go once it is back, so a meeting stopped offline completes by itself.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

/// Whether this Mac can reach Codex right now.
pub trait Reachability: Send + Sync + 'static {
    fn online(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
}

/// A `HEAD` request to the Codex endpoint through the HTTP stack the
/// requests use, so it goes through the system proxy and falls back from a
/// broken IPv6 route as they do. Any answer, whatever its status, means
/// online; a request that gets none means offline. HTTPS, so a captive
/// portal that intercepts it counts as offline.
pub struct CodexHost {
    /// A client is built for each check, as for each Codex request:
    /// reqwest reads the system proxy when it builds one, and the Mac may
    /// have changed networks since the last check.
    builder: Box<dyn Fn() -> reqwest::ClientBuilder + Send + Sync>,
    url: String,
}

impl CodexHost {
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
    const TIMEOUT: Duration = Duration::from_secs(5);

    pub(super) fn new() -> Self {
        Self::with(
            reqwest::Client::builder,
            crate::codex_transcribe::TRANSCRIBE_URL,
        )
    }

    fn with(
        builder: impl Fn() -> reqwest::ClientBuilder + Send + Sync + 'static,
        url: &str,
    ) -> Self {
        Self {
            builder: Box::new(builder),
            url: url.to_owned(),
        }
    }
}

impl Reachability for CodexHost {
    fn online(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async {
            let client = match (self.builder)()
                .connect_timeout(Self::CONNECT_TIMEOUT)
                .timeout(Self::TIMEOUT)
                .build()
            {
                Ok(client) => client,
                // Every Codex request would fail the same way: let the
                // queue go, and the run that follows finds out.
                Err(error) => {
                    log::warn!("Meeting connection check unavailable: {error}");
                    return true;
                }
            };
            client.head(&self.url).send().await.is_ok()
        })
    }
}

/// Codex was out of reach at the end of a run. Each run that ends so in a
/// row, with no answer between, waits twice as long before the connection
/// is checked again; while the check fails, it runs every minute, so the
/// queue goes on about a minute after the connection is back. Wall time,
/// like the rate-limit hold: `Instant` stops while the Mac sleeps.
#[derive(Debug, Default)]
pub struct OfflineHold {
    /// When the connection is checked next; `None` when not offline.
    retry_at: Option<SystemTime>,
    in_a_row: u32,
}

impl OfflineHold {
    const FIRST: Duration = Duration::from_secs(60);
    const CAP: Duration = Duration::from_secs(15 * 60);
    /// Between checks that found no connection.
    const RECHECK: Duration = Duration::from_secs(60);

    /// One more run could not reach Codex: returns the wait it starts.
    pub fn unreachable(&mut self, now: SystemTime) -> Duration {
        let wait = Self::FIRST
            .saturating_mul(1 << self.in_a_row.min(8))
            .min(Self::CAP);
        self.in_a_row += 1;
        self.retry_at = Some(now + wait);
        wait
    }

    /// Codex answered: the next unreachable run waits the shortest time.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Meetings not asked for wait.
    pub fn holds(&self) -> bool {
        self.retry_at.is_some()
    }

    /// When the connection is checked next.
    pub fn retry_at(&self) -> Option<SystemTime> {
        self.retry_at
    }

    /// The connection is due for a check.
    pub fn due(&self, now: SystemTime) -> bool {
        self.retry_at.is_some_and(|at| now >= at)
    }

    /// The check found the connection back, or not yet.
    pub fn checked(&mut self, online: bool, now: SystemTime) {
        self.retry_at = (!online).then_some(now + Self::RECHECK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    #[test]
    fn each_unreachable_run_in_a_row_waits_twice_as_long_up_to_fifteen_minutes() {
        let mut hold = OfflineHold::default();
        let minutes: Vec<u64> = (0..7)
            .map(|_| hold.unreachable(now()).as_secs() / 60)
            .collect();
        assert_eq!(minutes, [1, 2, 4, 8, 15, 15, 15]);
        assert_eq!(hold.retry_at(), Some(now() + 15 * MINUTE));
    }

    #[test]
    fn an_answer_resets_the_wait_and_lifts_the_hold() {
        let mut hold = OfflineHold::default();
        hold.unreachable(now());
        hold.unreachable(now());
        hold.reset();
        assert!(!hold.holds());
        assert_eq!(hold.unreachable(now()), MINUTE);
    }

    #[test]
    fn the_hold_is_due_for_a_check_once_its_wait_has_passed() {
        let mut hold = OfflineHold::default();
        assert!(!hold.holds());
        assert!(!hold.due(now()));
        hold.unreachable(now());
        assert!(hold.holds());
        assert!(!hold.due(now() + MINUTE - Duration::from_millis(1)));
        assert!(hold.due(now() + MINUTE));
    }

    #[test]
    fn a_failed_check_looks_again_a_minute_later_without_doubling() {
        let mut hold = OfflineHold::default();
        for _ in 0..4 {
            hold.unreachable(now());
        }
        let later = now() + 8 * MINUTE;
        hold.checked(false, later);
        assert_eq!(hold.retry_at(), Some(later + MINUTE));
        hold.checked(false, later + MINUTE);
        assert_eq!(hold.retry_at(), Some(later + 2 * MINUTE));
    }

    #[test]
    fn a_check_that_finds_the_connection_lifts_the_hold_but_keeps_the_count() {
        let mut hold = OfflineHold::default();
        hold.unreachable(now());
        hold.unreachable(now());
        hold.checked(true, now() + 2 * MINUTE);
        assert!(!hold.holds());
        // Codex was still out of reach on the run that followed.
        assert_eq!(hold.unreachable(now()), 4 * MINUTE);
    }

    /// Answers one request with `status` and returns its address and the
    /// request line it got.
    fn answer_once(status: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut header = String::new();
            while reader.read_line(&mut header).unwrap() > 2 {
                header.clear();
            }
            let response = format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n");
            reader.get_mut().write_all(response.as_bytes()).unwrap();
            sent.send(line.trim_end().to_owned()).unwrap();
        });
        (address, received)
    }

    #[tokio::test]
    async fn the_check_goes_through_the_proxy_the_requests_use() {
        // Only the proxy leads out: the host does not even resolve directly.
        let (proxy, request) = answer_once("403 Forbidden");
        let host = CodexHost::with(
            move || {
                reqwest::Client::builder()
                    .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
            },
            "http://codex.invalid/backend-api/transcribe",
        );
        assert!(host.online().await);
        assert_eq!(
            request.recv().unwrap(),
            "HEAD http://codex.invalid/backend-api/transcribe HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn each_check_takes_the_proxy_the_mac_has_now() {
        // The Mac moves from one network to another between two checks,
        // and the system proxy with it.
        let (first, first_request) = answer_once("403 Forbidden");
        let (second, second_request) = answer_once("403 Forbidden");
        let proxies = std::sync::Mutex::new(vec![second, first]);
        let host = CodexHost::with(
            move || {
                let proxy = proxies.lock().unwrap().pop().unwrap();
                reqwest::Client::builder()
                    .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
            },
            "http://codex.invalid/",
        );
        assert!(host.online().await);
        assert!(first_request.recv_timeout(Duration::from_secs(5)).is_ok());
        assert!(host.online().await);
        assert!(second_request.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[tokio::test]
    async fn any_answer_means_online_whatever_its_status() {
        let (server, _request) = answer_once("503 Service Unavailable");
        let host = CodexHost::with(
            || reqwest::Client::builder().no_proxy(),
            &format!("http://{server}/"),
        );
        assert!(host.online().await);
    }

    #[tokio::test]
    async fn no_answer_means_offline() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);
        let host = CodexHost::with(
            || reqwest::Client::builder().no_proxy(),
            &format!("http://{address}/"),
        );
        assert!(!host.online().await);
    }
}
