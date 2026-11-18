//! Live dictation through ChatGPT's dictation WebSocket: while the user
//! speaks, the audio the recorder captures goes to the server, and when the
//! recording stops the text comes from that stream, well before an upload of
//! the whole clip would answer. The upload (`codex_transcribe`) stays the
//! answer to everything that goes wrong here.
//!
//! The endpoint is undocumented. `protocol` holds its messages as they were
//! observed, `transcript` builds the text as the official client does,
//! `session` owns the socket, `decision` says when the stream's text is
//! used, and `health` keeps the dictations after a setback off the stream.

mod decision;
mod feed;
mod health;
mod live;
mod protocol;
mod session;
mod transcript;

#[cfg(test)]
pub(crate) mod fake_server;
#[cfg(test)]
mod tests;

use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use decision::NotOpened;
use health::{Health, Setback};
use session::{Credentials, Params};

pub(crate) use live::{pair, LiveDictation, Stop};

const STREAM_URL: &str = "wss://chatgpt.com/backend-api/dictation/stream";

static HEALTH: Health = Health::new();

static PARKED: Parked = Parked::new();

/// What a dictation that starts allows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Wanted<'a> {
    /// The `cloud_stream_enabled` setting.
    pub(crate) enabled: bool,
    /// ChatGPT is to transcribe this dictation.
    pub(crate) cloud: bool,
    pub(crate) signed_in: bool,
    /// The dictation language setting, "auto" included.
    pub(crate) language: &'a str,
}

/// The language a stream is started in, or why this dictation has none.
fn plan(wanted: &Wanted<'_>, setback: Option<Setback>) -> Result<Option<String>, NotOpened> {
    if !wanted.enabled {
        return Err(NotOpened::SettingOff);
    }
    if !wanted.cloud {
        return Err(NotOpened::OtherEngine);
    }
    if !wanted.signed_in {
        return Err(NotOpened::SignedOut);
    }
    if let Some(setback) = setback {
        return Err(NotOpened::Health(setback));
    }
    Ok(crate::codex_transcribe::request_language(wanted.language).map(str::to_string))
}

/// The stream of the dictation being recorded, between its start and its
/// stop. One dictation records at a time.
struct Parked {
    live: Mutex<Option<LiveDictation>>,
}

impl Parked {
    const fn new() -> Self {
        Self {
            live: Mutex::new(None),
        }
    }

    /// A stream still here from an earlier dictation is closed.
    fn park(&self, live: LiveDictation) {
        *self.slot() = Some(live);
    }

    fn take(&self) -> Option<LiveDictation> {
        self.slot().take()
    }

    fn settle(&self, plan: Result<Params, NotOpened>) {
        if let Some(live) = self.slot().as_mut() {
            live.settle(plan);
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<LiveDictation>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Keeps the stream of the recording that just started until its stop.
pub(crate) fn park(live: LiveDictation) {
    PARKED.park(live);
}

/// Opens the parked stream when this dictation allows one, and otherwise
/// drops the audio that waited for it. Does no I/O itself: the session runs
/// as a task.
pub(crate) fn open_parked(wanted: &Wanted<'_>) {
    let plan = plan(wanted, HEALTH.setback(SystemTime::now())).map(|language| Params {
        url: STREAM_URL.to_string(),
        credentials: Credentials::AuthFile(None),
        language,
    });
    PARKED.settle(plan);
}

/// The stream of the recording that just stopped.
pub(crate) fn take() -> Option<LiveDictation> {
    PARKED.take()
}

/// A cancel: the parked stream's socket is closed at once.
pub(crate) fn abort_parked() {
    drop(PARKED.take());
}
