//! Typed transcription failures. The kind decides the user-facing message and
//! whether a request is worth retrying; the detail is for logs only.

use serde::Serialize;
use specta::Type;
use std::fmt;
use std::time::Duration;

/// Extra attempts after the first request fails with a transient error.
pub const MAX_RETRIES: u32 = 2;
const RETRY_BACKOFF: [Duration; MAX_RETRIES as usize] =
    [Duration::from_millis(500), Duration::from_millis(1500)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The service could not be reached, or the connection dropped mid-request.
    Network,
    Timeout,
    RateLimited,
    ServerError,
    /// The session expired and could not be refreshed: the user must sign in.
    SessionExpired,
    /// No usable session on disk.
    SignedOut,
    TooLarge,
    /// Any other 4xx answer.
    Rejected,
    /// Local chosen with no ChatGPT account to take over: the model is not
    /// downloaded or not ready yet.
    LocalNotReady,
    /// Local chosen with no ChatGPT account to take over: the local run failed.
    LocalFailed,
    /// Local chosen with no ChatGPT account to take over: the dictation
    /// language is outside Parakeet's 25.
    LanguageNeedsCloud,
    Other,
}

impl FailureKind {
    pub fn from_status(status: u16) -> Self {
        match status {
            401 => Self::SessionExpired,
            413 => Self::TooLarge,
            429 => Self::RateLimited,
            500..=599 => Self::ServerError,
            _ => Self::Rejected,
        }
    }

    /// A connection that never opened is `Network` even when it timed out
    /// (no answer to DNS, the SYN or the TLS handshake, as on a Wi-Fi with no
    /// Internet): the service was never reached.
    pub fn from_transport(error: &reqwest::Error) -> Self {
        if error.is_connect() {
            Self::Network
        } else if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Network
        }
    }
}

/// Delay before retry number `attempt` (0-based), or `None` when the failure
/// is permanent or the retry budget is spent. Auth and 4xx answers never retry.
pub fn retry_delay(kind: FailureKind, attempt: u32) -> Option<Duration> {
    let transient = matches!(
        kind,
        FailureKind::Network | FailureKind::Timeout | FailureKind::ServerError
    );
    if !transient {
        return None;
    }
    RETRY_BACKOFF.get(attempt as usize).copied()
}

#[derive(Debug, Clone)]
pub struct TranscriptionError {
    pub kind: FailureKind,
    pub detail: String,
}

impl TranscriptionError {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    /// The kind of an error chain, `Other` when nothing classified it.
    pub fn kind_of(error: &anyhow::Error) -> FailureKind {
        error
            .downcast_ref::<Self>()
            .map(|error| error.kind)
            .unwrap_or(FailureKind::Other)
    }
}

impl fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for TranscriptionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_kinds() {
        assert_eq!(FailureKind::from_status(401), FailureKind::SessionExpired);
        assert_eq!(FailureKind::from_status(403), FailureKind::Rejected);
        assert_eq!(FailureKind::from_status(400), FailureKind::Rejected);
        assert_eq!(FailureKind::from_status(413), FailureKind::TooLarge);
        assert_eq!(FailureKind::from_status(429), FailureKind::RateLimited);
        assert_eq!(FailureKind::from_status(500), FailureKind::ServerError);
        assert_eq!(FailureKind::from_status(503), FailureKind::ServerError);
    }

    #[test]
    fn transient_failures_retry_twice_with_backoff() {
        for kind in [
            FailureKind::Network,
            FailureKind::Timeout,
            FailureKind::ServerError,
        ] {
            assert_eq!(retry_delay(kind, 0), Some(Duration::from_millis(500)));
            assert_eq!(retry_delay(kind, 1), Some(Duration::from_millis(1500)));
            assert_eq!(retry_delay(kind, 2), None);
        }
    }

    #[test]
    fn auth_and_client_errors_never_retry() {
        for kind in [
            FailureKind::SessionExpired,
            FailureKind::SignedOut,
            FailureKind::RateLimited,
            FailureKind::TooLarge,
            FailureKind::Rejected,
            FailureKind::Other,
        ] {
            assert_eq!(retry_delay(kind, 0), None, "{kind:?} must not retry");
        }
    }

    fn quick_client_builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(100))
            .timeout(Duration::from_millis(1000))
            .no_proxy()
    }

    fn quick_client() -> reqwest::Client {
        quick_client_builder().build().unwrap()
    }

    /// A DNS server that never answers, as on a Wi-Fi with no Internet behind
    /// it. Local, so the test does not depend on how the network treats an
    /// unreachable address.
    struct SilentDns;

    impl reqwest::dns::Resolve for SilentDns {
        fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn a_connection_that_never_opens_is_a_network_failure() {
        let error = quick_client_builder()
            .dns_resolver(SilentDns)
            .build()
            .unwrap()
            .get("https://murmur.invalid/")
            .send()
            .await
            .unwrap_err();
        assert!(error.is_connect() && error.is_timeout(), "{error}");
        assert_eq!(FailureKind::from_transport(&error), FailureKind::Network);
    }

    #[tokio::test]
    async fn an_answer_that_never_comes_is_a_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _socket = listener.accept().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let error = quick_client().get(url).send().await.unwrap_err();
        assert!(!error.is_connect() && error.is_timeout(), "{error}");
        assert_eq!(FailureKind::from_transport(&error), FailureKind::Timeout);
    }

    #[test]
    fn kind_survives_anyhow_context() {
        let error = anyhow::Error::new(TranscriptionError::new(FailureKind::Timeout, "slow"))
            .context("Codex transcription failed");
        assert_eq!(TranscriptionError::kind_of(&error), FailureKind::Timeout);
        assert_eq!(
            TranscriptionError::kind_of(&anyhow::anyhow!("plain")),
            FailureKind::Other
        );
    }
}
