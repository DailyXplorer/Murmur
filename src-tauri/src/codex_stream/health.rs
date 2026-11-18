//! What the last streams taught about the service, so the dictations that
//! follow a setback go straight to the upload instead of paying for it too.
//! Process memory only.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// Why the stream is left alone for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Setback {
    /// The server refused the handshake.
    Refused,
    /// A stream went wrong in any other way.
    Failed,
}

impl Setback {
    /// A refusal is the server's answer to this account or this client, and
    /// stands until something changes on its side: an hour, as for a spent
    /// dictation quota. Any other failure may be the network of the moment:
    /// five minutes.
    fn lasts(self) -> Duration {
        match self {
            Self::Refused => Duration::from_secs(60 * 60),
            Self::Failed => Duration::from_secs(5 * 60),
        }
    }
}

pub(crate) struct Health {
    setback: Mutex<Option<(Setback, SystemTime)>>,
}

impl Health {
    pub(crate) const fn new() -> Self {
        Self {
            setback: Mutex::new(None),
        }
    }

    /// The setback that keeps the stream off at `now`.
    pub(crate) fn setback(&self, now: SystemTime) -> Option<Setback> {
        let (setback, since) = (*self.state())?;
        crate::codex_transcribe::within(Some(since), now, setback.lasts()).then_some(setback)
    }

    pub(crate) fn note(&self, setback: Setback, now: SystemTime) {
        *self.state() = Some((setback, now));
    }

    /// A stream gave a text.
    pub(crate) fn clear(&self) {
        *self.state() = None;
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Option<(Setback, SystemTime)>> {
        self.setback.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    #[test]
    fn nothing_keeps_a_first_stream_off() {
        assert_eq!(Health::new().setback(at(0)), None);
    }

    #[test]
    fn a_refusal_keeps_the_stream_off_for_an_hour() {
        let health = Health::new();
        health.note(Setback::Refused, at(0));
        assert_eq!(health.setback(at(0)), Some(Setback::Refused));
        assert_eq!(health.setback(at(60 * 60 - 1)), Some(Setback::Refused));
        assert_eq!(health.setback(at(60 * 60)), None);
    }

    #[test]
    fn a_failure_keeps_the_stream_off_for_five_minutes() {
        let health = Health::new();
        health.note(Setback::Failed, at(0));
        assert_eq!(health.setback(at(5 * 60 - 1)), Some(Setback::Failed));
        assert_eq!(health.setback(at(5 * 60)), None);
    }

    #[test]
    fn a_stream_that_gave_a_text_clears_the_setback() {
        let health = Health::new();
        health.note(Setback::Refused, at(0));
        health.clear();
        assert_eq!(health.setback(at(1)), None);
    }

    #[test]
    fn a_clock_set_back_ends_the_wait() {
        let health = Health::new();
        health.note(Setback::Failed, at(100));
        assert_eq!(health.setback(at(99)), None);
    }
}
