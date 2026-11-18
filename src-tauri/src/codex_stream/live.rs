//! One dictation's stream as the rest of the app holds it: made with the
//! recorder's tap, opened or not once the recording runs, and asked for its
//! text when the recording stops.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use log::info;
use tokio::sync::watch;
use tokio::time::Instant;

use super::decision::{decide, Decision, Facts, Failure, NotOpened, Skip, Standing};
use super::feed::{self, Audio, Meter};
use super::health::Health;
use super::session::{self, Params, State};
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::FrameTap;
use crate::ProcessingOperation;

/// With this much audio still unsent when the recording stops, the uplink
/// is slower than speech: the upload, a third of the size, is the faster
/// way.
const MAX_UNSENT_AT_STOP: usize = 3 * TARGET_SAMPLE_RATE as usize;

/// How long the stop waits for each step of a session.
#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    pub(super) start: Duration,
    pub(super) flush: Duration,
    pub(super) close: Duration,
}

impl Timing {
    pub(super) const MEASURED: Self = Self {
        // The handshake took about 0.45 s, less than a short dictation and
        // its release tail. A session still not started this long after the
        // stop is far slower than that.
        start: Duration::from_millis(400),
        // What is left at the stop is at most `MAX_UNSENT_AT_STOP`, about
        // 130 kB as base64: a second sends it on any uplink the stream is
        // worth using on, and a stalled socket is not waited for.
        flush: Duration::from_secs(1),
        // The final text came 0.5 to 0.8 s after `session.close`, 1.64 s
        // at most over about 70 sessions.
        close: Duration::from_millis(2500),
    };
}

/// What the pipeline knows of a dictation when its recording stopped.
pub(crate) struct Stop<'a> {
    /// Quiet speech was raised for the engine.
    pub(crate) audio_raised: bool,
    /// The language of this dictation as a request names it.
    pub(crate) language: Option<&'a str>,
}

/// The stream of one dictation. Dropping it closes its socket at once,
/// without `session.close`.
pub(crate) struct LiveDictation {
    stream: Stream,
}

enum Stream {
    /// Made with the tap: the frames wait until something decides.
    Undecided(Audio),
    NotOpened(NotOpened),
    Open(Open),
}

struct Open {
    language: Option<String>,
    state: watch::Receiver<State>,
    meter: Arc<Meter>,
    aborted: Arc<AtomicBool>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.aborted.store(true, Ordering::Release);
        self.task.abort();
    }
}

/// The tap to give the recorder, and the stream that will hear it. Nothing
/// is opened yet.
pub(crate) fn pair() -> (FrameTap, LiveDictation) {
    let (mut feed, audio) = feed::channel();
    let live = LiveDictation {
        stream: Stream::Undecided(audio),
    };
    (Box::new(move |frame: &[f32]| feed.push(frame)), live)
}

impl LiveDictation {
    /// Opens the session, or drops the frames that waited for it. Only the
    /// first call decides.
    pub(super) fn settle(&mut self, plan: Result<Params, NotOpened>) {
        let undecided = Stream::NotOpened(NotOpened::Undecided);
        self.stream = match (std::mem::replace(&mut self.stream, undecided), plan) {
            (Stream::Undecided(audio), Ok(params)) => Stream::Open(Open::start(params, audio)),
            (Stream::Undecided(_), Err(reason)) => Stream::NotOpened(reason),
            (decided, _) => decided,
        };
    }

    /// The text of the dictation, when the stream's can be used. `None`
    /// sends the audio to the upload. Each stream is asked once.
    pub(crate) async fn text(
        self,
        stop: &Stop<'_>,
        operation: &ProcessingOperation,
    ) -> Option<String> {
        self.text_with(stop, operation, Timing::MEASURED, &super::HEALTH)
            .await
    }

    pub(super) async fn text_with(
        self,
        stop: &Stop<'_>,
        operation: &ProcessingOperation,
        timing: Timing,
        health: &Health,
    ) -> Option<String> {
        let asked = Instant::now();
        match self.outcome(stop, operation, timing).await {
            Ok(text) => {
                health.clear();
                info!(
                    "Cloud stream gave the text {} ms after it was asked",
                    asked.elapsed().as_millis()
                );
                Some(text)
            }
            Err(skip) => {
                if let Some(setback) = skip.setback() {
                    health.note(setback, SystemTime::now());
                }
                info!("Cloud stream not used: {skip}");
                None
            }
        }
    }

    /// Decides with what is known, waits for the session when nothing rules
    /// it out, and decides again. The stream is closed on the way out.
    pub(super) async fn outcome(
        mut self,
        stop: &Stop<'_>,
        operation: &ProcessingOperation,
        timing: Timing,
    ) -> Result<String, Skip> {
        let asked = Instant::now();
        let (not_opened, language_changed) = match &self.stream {
            Stream::Undecided(_) => (Some(NotOpened::Undecided), false),
            Stream::NotOpened(reason) => (Some(*reason), false),
            Stream::Open(open) => (None, open.language.as_deref() != stop.language),
        };
        let mut facts = Facts {
            not_opened,
            cancelled: operation.cancellation_requested(),
            audio_raised: stop.audio_raised,
            language_changed,
            stream: Standing::Unknown,
        };
        loop {
            match decide(&facts) {
                Decision::Use(text) => return Ok(text),
                Decision::Skip(skip) => return Err(skip),
                Decision::Wait => {
                    // `decide` skips a stream that is not open before it
                    // would wait for it.
                    let Stream::Open(open) = &mut self.stream else {
                        return Err(Skip::NotOpened(NotOpened::Undecided));
                    };
                    facts.stream = open.standing(asked, timing, operation).await;
                    facts.cancelled = operation.cancellation_requested();
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> Option<watch::Receiver<State>> {
        match &self.stream {
            Stream::Open(open) => Some(open.state.clone()),
            Stream::Undecided(_) | Stream::NotOpened(_) => None,
        }
    }
}

impl Open {
    fn start(params: Params, audio: Audio) -> Self {
        let (state_tx, state) = watch::channel(State::Connecting);
        let aborted = Arc::new(AtomicBool::new(false));
        let language = params.language.clone();
        let meter = Arc::clone(&audio.meter);
        let task = tauri::async_runtime::spawn(session::run(
            params,
            audio,
            Arc::clone(&aborted),
            state_tx,
        ));
        Self {
            language,
            state,
            meter,
            aborted,
            task,
        }
    }

    /// Where the session stands, waiting for it no longer than `timing`
    /// allows from `asked`. `Unknown` when the operation was cancelled
    /// meanwhile.
    async fn standing(
        &mut self,
        asked: Instant,
        timing: Timing,
        operation: &ProcessingOperation,
    ) -> Standing {
        if let State::Done(end) = &*self.state.borrow() {
            return end.clone().into();
        }
        if self.meter.overflowed() || self.meter.unsent_at_stop().unwrap_or(0) > MAX_UNSENT_AT_STOP
        {
            return Standing::Lagging;
        }
        loop {
            let state = self.state.borrow_and_update().clone();
            let (deadline, late) = match state {
                State::Done(end) => return end.into(),
                State::Connecting => (asked + timing.start, Standing::NotStarted),
                State::Streaming { since } => (since.max(asked) + timing.flush, Standing::Lagging),
                State::Closing { since } => (since + timing.close, Standing::NotClosed),
            };
            tokio::select! {
                changed = self.state.changed() => {
                    if changed.is_err() {
                        return Standing::Failed(Failure::ClosedEarly);
                    }
                }
                _ = tokio::time::sleep_until(deadline) => return late,
                _ = operation.cancelled() => return Standing::Unknown,
            }
        }
    }
}
