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

/// A TCP connection to the Codex host: it needs DNS, a route and the
/// server, and sends nothing.
pub struct CodexHost;

impl CodexHost {
    const ADDRESS: &'static str = "chatgpt.com:443";
    const TIMEOUT: Duration = Duration::from_secs(3);
}

impl Reachability for CodexHost {
    fn online(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async {
            let connect = tokio::net::TcpStream::connect(Self::ADDRESS);
            matches!(
                tokio::time::timeout(Self::TIMEOUT, connect).await,
                Ok(Ok(_))
            )
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
}
