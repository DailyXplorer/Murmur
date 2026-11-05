use super::{MeetingProgress, ProcessEvents};
use crate::meetings::chunking::split_chunk;
use crate::meetings::merge::Track;
use crate::meetings::store::{MeetingStage, MeetingsStore};
use crate::meetings::track_audio::{self, TrackLevels};
use crate::meetings::transcribe::{
    unreachable, ChunkSource, CodexVerdict, Job, Outcome, RunObserver,
};
use crate::transcription_error::FailureKind;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub(super) struct Audio {
    pub(super) path: PathBuf,
    pub(super) levels: TrackLevels,
    pub(super) threshold: Option<f32>,
}

pub(super) struct TrackFiles(pub(super) HashMap<Track, Audio>);

impl ChunkSource for TrackFiles {
    fn samples(&self, track: Track, start_ms: u64, end_ms: u64) -> anyhow::Result<Vec<f32>> {
        let audio = self
            .0
            .get(&track)
            .ok_or_else(|| anyhow::anyhow!("No {track:?} track"))?;
        track_audio::read_range(&audio.path, start_ms, end_ms)
    }

    fn split(&self, track: Track, start_ms: u64, end_ms: u64) -> Option<u64> {
        let audio = self.0.get(&track)?;
        split_chunk(&audio.levels.levels, start_ms, end_ms).map(|(_, cut, _)| cut)
    }
}

/// Saves each piece as it arrives and reports progress.
pub(super) struct Saver {
    pub(super) id: i64,
    pub(super) store: Arc<MeetingsStore>,
    pub(super) events: Arc<dyn ProcessEvents>,
    pub(super) done: AtomicU32,
    pub(super) total: u32,
    pub(super) rate_limited: AtomicBool,
    /// A piece ran out of network retries.
    pub(super) unreachable: AtomicBool,
    /// Codex transcribed at least one piece in this run.
    pub(super) answered: AtomicBool,
}

impl Saver {
    /// A piece that ran out of rate-limit budget holds the queue, whatever
    /// else ended the run and however many pieces Codex answered first; one
    /// that ran out of network retries holds it until the connection is back.
    pub(super) fn verdict(&self) -> CodexVerdict {
        if self.rate_limited.load(Ordering::SeqCst) {
            CodexVerdict::RateLimited
        } else if self.unreachable.load(Ordering::SeqCst) {
            CodexVerdict::Unreachable
        } else if self.answered.load(Ordering::SeqCst) {
            CodexVerdict::Answered
        } else {
            CodexVerdict::Unknown
        }
    }

    /// Notes why Codex gave up on a piece, when that holds the queue. A
    /// piece ends with a network failure only once its retries are spent.
    fn codex_out(&self, kind: FailureKind) {
        if kind == FailureKind::RateLimited {
            self.rate_limited.store(true, Ordering::SeqCst);
        } else if unreachable(kind) {
            self.unreachable.store(true, Ordering::SeqCst);
        }
    }

    pub(super) fn report(&self, rate_limited: bool) {
        self.events.progress(MeetingProgress {
            id: self.id,
            stage: MeetingStage::Transcribing,
            chunks_done: self.done.load(Ordering::SeqCst),
            chunks_total: self.total,
            rate_limited,
        });
    }
}

impl RunObserver for Saver {
    fn finished(&self, job: &Job, outcome: &Outcome) {
        let saved = match outcome {
            Outcome::Done(text) => {
                self.done.fetch_add(1, Ordering::SeqCst);
                self.answered.store(true, Ordering::SeqCst);
                self.store.chunk_done(self.id, job.track, job.idx, text)
            }
            Outcome::DoneLocally { text, codex } => {
                self.done.fetch_add(1, Ordering::SeqCst);
                self.codex_out(*codex);
                self.store.chunk_done_locally(
                    self.id,
                    job.track,
                    job.idx,
                    text,
                    &format!("{codex:?}"),
                )
            }
            Outcome::Failed(kind) => {
                self.codex_out(*kind);
                self.store
                    .chunk_failed(self.id, job.track, job.idx, &format!("{kind:?}"))
            }
        };
        if let Err(error) = saved {
            log::error!(
                "Meeting {}: cannot save a transcribed piece: {error}",
                self.id
            );
        }
        self.report(false);
    }

    fn rate_limited(&self) {
        self.report(true);
    }
}
