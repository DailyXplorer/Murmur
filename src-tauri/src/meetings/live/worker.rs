use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{self, error::TryRecvError};

use super::chunker::{Chunk, Cut, RATE};
use super::echo::{is_echo, ECHO_WINDOW_MS};
use super::seam::{heard, Draft, HeardWord};
use super::{draft, Live, LiveEntry, LiveEntryKind, LiveState, LiveUnavailable};
use crate::engine::local::session::{Background, BoxFuture};
use crate::meetings::merge::Track as Source;

const RETRY: Duration = Duration::from_millis(250);
const CATCHING_UP_MS: u64 = 30_000;
const MAX_BACKLOG_MS: u64 = 120_000;
const MAX_FAILURES: u32 = 3;
const SYSTEM_MEMORY_MS: u64 = 60_000;

/// The local model as the live transcript uses it.
pub trait LiveEngine: Send + Sync + 'static {
    /// Whether the model can transcribe now.
    fn availability(&self) -> Result<(), LiveUnavailable>;
    /// Loads the model ahead of the first piece.
    fn warm(&self);
    fn transcribe<'a>(&'a self, samples: &'a [f32]) -> BoxFuture<'a, Background>;
    fn keep_loaded(&self);
}

fn ms(samples: u64) -> u64 {
    samples * 1_000 / RATE as u64
}

/// Where the worker's results go.
pub struct Out {
    pub id: i64,
    pub dir: PathBuf,
    pub live: Arc<Live>,
    pub discarded: Arc<AtomicBool>,
}

impl Out {
    fn send(&self, state: LiveState, entries: Vec<LiveEntry>) {
        if self.discarded.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = draft::append(&self.dir, &entries) {
            log::warn!(
                "Meeting {}: cannot save the live transcript: {error}",
                self.id
            );
        }
        self.live.update(self.id, state, entries);
    }
}

pub struct Worker {
    out: Out,
    engine: Arc<dyn LiveEngine>,
    mic: Draft,
    system: Draft,
    pending: VecDeque<Chunk>,
    held: Vec<LiveEntry>,
    system_heard: Vec<LiveEntry>,
    has_system: bool,
    headphones: Arc<AtomicBool>,
    failures: u32,
    spoken: bool,
    state: LiveState,
}

impl Worker {
    pub fn new(
        out: Out,
        engine: Arc<dyn LiveEngine>,
        has_system: bool,
        headphones: Arc<AtomicBool>,
    ) -> Self {
        Self {
            out,
            engine,
            mic: Draft::new(Source::Mic),
            system: Draft::new(Source::System),
            pending: VecDeque::new(),
            held: Vec::new(),
            system_heard: Vec::new(),
            has_system,
            headphones,
            failures: 0,
            spoken: false,
            state: LiveState::Starting,
        }
    }

    pub async fn run(mut self, mut chunks: mpsc::Receiver<Chunk>) {
        loop {
            let open = self.take(&mut chunks);
            if self.pending.is_empty() {
                if !open {
                    break;
                }
                match chunks.recv().await {
                    Some(chunk) => self.pending.push_back(chunk),
                    None => break,
                }
                continue;
            }
            self.skip_backlog();
            self.report(Vec::new());
            if let Some(chunk) = self.pending.pop_front() {
                self.process(chunk, &mut chunks).await;
            }
        }
        let mut entries = self.mic.finish();
        entries.extend(self.system.finish());
        self.state = LiveState::Stopped;
        self.route(entries, true);
        log::info!("Meeting {}: live transcript stopped", self.out.id);
    }

    fn take(&mut self, chunks: &mut mpsc::Receiver<Chunk>) -> bool {
        loop {
            match chunks.try_recv() {
                Ok(chunk) => self.pending.push_back(chunk),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn backlog_ms(&self) -> u64 {
        self.pending
            .iter()
            .filter(|chunk| !chunk.silent())
            .map(|chunk| ms((chunk.samples.len() - chunk.overlap) as u64))
            .sum()
    }

    fn skip_backlog(&mut self) {
        let mut skipped: Vec<(Source, u64, u64)> = Vec::new();
        while self.backlog_ms() > MAX_BACKLOG_MS {
            let Some(chunk) = self.pending.pop_front() else {
                break;
            };
            let range = (ms(chunk.start_sample), ms(chunk.end_sample()));
            match skipped
                .iter_mut()
                .find(|(source, ..)| *source == chunk.source)
            {
                Some((_, _, end)) => *end = range.1,
                None => skipped.push((chunk.source, range.0, range.1)),
            }
        }
        for (source, start, end) in skipped {
            log::warn!(
                "Meeting {}: the live transcript skipped {} s",
                self.out.id,
                (end - start) / 1_000
            );
            let entries = self.draft(source).gap(start, end);
            self.route(entries, false);
        }
    }

    fn draft(&mut self, source: Source) -> &mut Draft {
        match source {
            Source::Mic => &mut self.mic,
            Source::System => &mut self.system,
        }
    }

    fn current_state(&self) -> LiveState {
        if self.failures >= MAX_FAILURES {
            return LiveState::Unavailable {
                reason: LiveUnavailable::EngineFailed,
            };
        }
        if let Err(reason) = self.engine.availability() {
            return LiveState::Unavailable { reason };
        }
        if self.backlog_ms() > CATCHING_UP_MS {
            LiveState::CatchingUp
        } else if self.spoken {
            LiveState::Live
        } else {
            LiveState::Starting
        }
    }

    fn report(&mut self, entries: Vec<LiveEntry>) {
        if self.state != LiveState::Stopped {
            self.state = self.current_state();
        }
        self.spoken |= entries
            .iter()
            .any(|entry| entry.kind == LiveEntryKind::Speech);
        if self.state == LiveState::Starting && self.spoken {
            self.state = LiveState::Live;
        }
        self.out.send(self.state, entries);
    }

    async fn process(&mut self, chunk: Chunk, chunks: &mut mpsc::Receiver<Chunk>) {
        self.engine.keep_loaded();
        let stopped = self.failures >= MAX_FAILURES || self.engine.availability().is_err();
        if stopped || chunk.silent() {
            self.settle(&chunk, Vec::new());
            return;
        }
        loop {
            match self.engine.transcribe(&chunk.samples).await {
                Background::Done(words) => {
                    self.failures = 0;
                    let words = heard(&words, ms(chunk.start_sample));
                    self.settle(&chunk, words);
                    return;
                }
                Background::Busy => {
                    tokio::time::sleep(RETRY).await;
                    self.take(chunks);
                }
                Background::Failed(error) => {
                    self.failures += 1;
                    log::warn!(
                        "Meeting {}: a live piece failed ({} in a row): {error:#}",
                        self.out.id,
                        self.failures
                    );
                    let entries = self
                        .draft(chunk.source)
                        .gap(ms(chunk.start_sample), ms(chunk.end_sample()));
                    self.route(entries, false);
                    return;
                }
            }
        }
    }

    fn settle(&mut self, chunk: &Chunk, words: Vec<HeardWord>) {
        let draft = self.draft(chunk.source);
        draft.accept(
            ms(chunk.start_sample),
            ms(chunk.overlap as u64),
            words,
            ms(chunk.settled_sample()),
            chunk.cut == Cut::Hard,
        );
        let entries = draft.publish();
        self.route(entries, false);
    }

    fn route(&mut self, entries: Vec<LiveEntry>, end: bool) {
        let hold = self.has_system && !self.headphones.load(Ordering::Relaxed);
        let mut out = Vec::new();
        for entry in entries {
            match entry.source {
                Source::System => {
                    self.system_heard.push(entry.clone());
                    out.push(entry);
                }
                Source::Mic if hold => self.held.push(entry),
                Source::Mic => out.push(entry),
            }
        }
        let heard_until = if end {
            u64::MAX
        } else {
            self.system.settled_ms()
        };
        let (due, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|entry| !hold || entry.end_ms + ECHO_WINDOW_MS <= heard_until);
        self.held = held;
        for entry in due {
            if entry.kind == LiveEntryKind::Speech && hold && is_echo(&entry, &self.system_heard) {
                continue;
            }
            out.push(entry);
        }
        if let Some(last) = self.system_heard.last().map(|entry| entry.end_ms) {
            self.system_heard
                .retain(|entry| entry.end_ms + SYSTEM_MEMORY_MS >= last);
        }
        self.report(out);
    }
}
