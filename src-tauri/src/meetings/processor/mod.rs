//! Turns a recorded meeting into its transcript, in stages that each save
//! their result first, so a quit or crash resumes where it stopped and no
//! piece already transcribed is sent again:
//!
//! 1. Transcribing: each track is split into pieces (`chunking`), and the
//!    pieces Codex has not yet transcribed are sent (`transcribe`). At the
//!    same time the on-device analysis (`analysis`) finds the time of each
//!    word heard and who spoke when.
//! 2. Aligning: each piece's words get the times of the words heard on
//!    device (`align`), or, when those cannot be trusted, its sentences are
//!    spread over the moments the track holds speech (`spread`). Words then
//!    go to their speaker (`speakers`), echoes of remote voices are taken off
//!    the microphone (`bleed`) unless the meeting played on headphones
//!    throughout, and both tracks are interleaved into bubbles
//!    (`merge`). Parts that failed show as placeholders, so the user gets
//!    everything that worked.
//! 3. Once the whole transcript is there, Codex writes the summary and the
//!    title (`summary`), and the tracks are compressed into one small file
//!    (`archive`). A meeting transcribed again gets its tracks back from it.

mod saver;
#[cfg(test)]
mod tests;
mod transcript;

use super::analysis::{diarized_track, Analysis, AnalysisRequest, Analyzer, AnalyzerOutcome};
use super::announce::{Finished, Outcome as FinishedOutcome};
use super::chunking::{plan_chunks, speech_threshold, system_audio_missing, voiced_ms};
use super::merge::Track;
use super::store::{
    ChunkStatus, MeetingStage, MeetingStatus, MeetingTracks, MeetingsStore, ProcessingError,
    SpeakerAnalysis,
};
use super::summary::{self, Summarizer};
use super::track_audio;
use super::transcribe::{
    ChunkSource, CodexVerdict, Job, RetryPolicy, RunEnd, RunObserver, Runner, Transcriber,
};
use super::{archive, paths};
use crate::ProcessingOperation;
use saver::{Audio, Saver, TrackFiles};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use transcript::{build_transcript, timings_line, untranscribed_ms};

/// How far processing has come, for the progress bar.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type, tauri_specta::Event)]
pub struct MeetingProgress {
    pub id: i64,
    pub stage: MeetingStage,
    /// Pieces with speech that are transcribed, out of `chunks_total`.
    pub chunks_done: u32,
    pub chunks_total: u32,
    /// Codex asked to slow down; processing waits and resumes on its own.
    pub rate_limited: bool,
}

pub trait ProcessEvents: Send + Sync + 'static {
    fn progress(&self, progress: MeetingProgress);
    /// The meeting's row or transcript changed.
    fn updated(&self, id: i64);
    fn finished(&self, finished: Finished);
}

pub struct Processor {
    pub store: Arc<MeetingsStore>,
    pub root: PathBuf,
    pub transcriber: Arc<dyn Transcriber>,
    /// Takes the pieces Codex refuses for good; `None` without a ready model.
    pub local: Option<Arc<dyn Transcriber>>,
    pub analyzer: Arc<dyn Analyzer>,
    pub summarizer: Arc<dyn Summarizer>,
    pub policy: RetryPolicy,
    pub language: Option<String>,
    /// The user's custom words, read when the job starts; the summary
    /// spells names and terms as they do.
    pub vocabulary: Vec<String>,
    pub events: Arc<dyn ProcessEvents>,
    /// Whether the tracks are compressed once the summary is written. Always
    /// on in the app; a test with a long recording turns it off, as encoding
    /// 25 minutes in a debug build takes minutes.
    pub compress_audio: bool,
}

fn track_file(track: Track) -> &'static str {
    match track {
        Track::Mic => paths::MIC_TRACK,
        Track::System => paths::SYSTEM_TRACK,
    }
}

/// System audio with this much speech, in milliseconds, is a side of the
/// conversation however long the user spoke: a few minutes of questions
/// after an hour of presenting. A ringtone or notifications stay far below.
const REMOTE_SPEECH_MS: u64 = 60_000;

/// System audio with less speech than that, and less than this share of
/// the microphone's, in percent, is not a side of the conversation: a
/// notification or a ringtone during a meeting held in person.
const MIN_REMOTE_SPEECH_PERCENT: u64 = 10;

/// The track whose voices are told apart when no words were heard on device
/// to pick it (`analysis::diarized_track`): system audio, where the remote
/// participants are, unless it holds too little speech for that, both in
/// time and next to the microphone's. Then the meeting was held in person
/// and every voice is on the microphone.
fn track_by_levels(audio: &HashMap<Track, Audio>) -> Track {
    let speech = |track| {
        audio.get(&track).map(|audio| {
            voiced_ms(
                &audio.levels.levels,
                audio.threshold,
                0,
                audio.levels.duration_ms,
            )
        })
    };
    match (speech(Track::Mic), speech(Track::System)) {
        (Some(mic), Some(system))
            if system >= REMOTE_SPEECH_MS || system * 100 > mic * MIN_REMOTE_SPEECH_PERCENT =>
        {
            Track::System
        }
        (Some(_), _) => Track::Mic,
        (None, _) => Track::System,
    }
}

fn recorded_tracks(recorded: MeetingTracks) -> Vec<Track> {
    [
        (recorded.has_mic(), Track::Mic),
        (recorded.has_system(), Track::System),
    ]
    .into_iter()
    .filter_map(|(recorded, track)| recorded.then_some(track))
    .collect()
}

/// Reads the levels of the recorded tracks, first bringing them back from
/// the compressed audio when they were deleted after an earlier run. A track
/// that cannot be read is left out.
fn read_tracks(dir: &Path, recorded: MeetingTracks) -> HashMap<Track, Audio> {
    if let Err(error) = archive::restore(dir, recorded) {
        log::warn!("Cannot restore the meeting tracks: {error}");
    }
    recorded_tracks(recorded)
        .iter()
        .filter_map(|track| {
            let path = dir.join(track_file(*track));
            match track_audio::levels(&path) {
                Ok(levels) => Some((
                    *track,
                    Audio {
                        threshold: speech_threshold(&levels.levels),
                        path,
                        levels,
                    },
                )),
                Err(error) => {
                    log::warn!("Cannot read a meeting track: {error}");
                    None
                }
            }
        })
        .collect()
}

impl Processor {
    pub(super) fn finish_processing(
        &self,
        id: i64,
        error: Option<ProcessingError>,
    ) -> anyhow::Result<()> {
        if self.store.finish_processing(id, error)? {
            self.events.updated(id);
            self.events.finished(Finished {
                id,
                outcome: if error.is_some() {
                    FinishedOutcome::Failed
                } else {
                    FinishedOutcome::Ready
                },
            });
        }
        Ok(())
    }

    /// Processes meeting `id` as far as it can: its transcript, then once
    /// that is complete its summary and compressed audio. Each step is
    /// skipped when already done, so this does nothing for a meeting that is
    /// gone, still recording or done. Returns what Codex said meanwhile.
    pub async fn process(
        &self,
        id: i64,
        operation: ProcessingOperation,
    ) -> anyhow::Result<CodexVerdict> {
        let Some(meeting) = self.store.get(id)? else {
            return Ok(CodexVerdict::Unknown);
        };
        let started = Instant::now();
        let mut stages = Vec::new();
        let mut codex = CodexVerdict::Unknown;
        if meeting.status != MeetingStatus::Ready {
            (stages, codex) = self.transcribe(id, operation.clone()).await?;
        }
        let ready = self
            .store
            .get(id)?
            .is_some_and(|meeting| meeting.status == MeetingStatus::Ready);
        // This Mac transcribed what Codex could not be reached for; the
        // summary waits for the connection instead of spending its retries.
        if ready && !operation.is_cancelled() && codex != CodexVerdict::Unreachable {
            codex = codex.max(self.finish(id, &operation, &mut stages).await?);
        }
        if !stages.is_empty() {
            let line = timings_line(
                meeting.duration_ms.max(0) as u64,
                &stages,
                started.elapsed(),
            );
            log::info!("Meeting {id} timings: {line}");
        }
        Ok(codex)
    }

    /// The summary, then the compressed audio.
    async fn finish(
        &self,
        id: i64,
        operation: &ProcessingOperation,
        stages: &mut Vec<(&'static str, Duration)>,
    ) -> anyhow::Result<CodexVerdict> {
        let started = Instant::now();
        let summarized = summary::write(
            &self.store,
            self.summarizer.as_ref(),
            &self.policy,
            id,
            summary::Hints {
                language: self.language.as_deref(),
                vocabulary: &self.vocabulary,
            },
            operation,
        )
        .await?;
        stages.push(("summary", started.elapsed()));
        self.events.updated(id);
        if !summarized.settled {
            return Ok(summarized.codex);
        }
        if !self.compress_audio {
            return Ok(summarized.codex);
        }
        let started = Instant::now();
        let (store, root) = (Arc::clone(&self.store), self.root.clone());
        let archived =
            tokio::task::spawn_blocking(move || archive::archive(&store, &root, id)).await?;
        stages.push(("compression", started.elapsed()));
        match archived {
            Ok(()) => self.events.updated(id),
            Err(error) => log::warn!("Meeting {id}: audio kept uncompressed: {error}"),
        }
        Ok(summarized.codex)
    }

    /// The transcript, from the parts Codex has not transcribed yet, how
    /// long each stage took and what Codex said.
    async fn transcribe(
        &self,
        id: i64,
        operation: ProcessingOperation,
    ) -> anyhow::Result<(Vec<(&'static str, Duration)>, CodexVerdict)> {
        if !self.store.begin_processing(id)? {
            return Ok((Vec::new(), CodexVerdict::Unknown));
        }
        self.events.updated(id);
        let Some(meeting) = self.store.get(id)? else {
            return Ok((Vec::new(), CodexVerdict::Unknown));
        };
        let recorded = meeting.tracks;
        let dir = paths::meeting_dir(&self.root, id);
        let started = Instant::now();
        let audio = tokio::task::spawn_blocking(move || read_tracks(&dir, recorded)).await?;
        let mut stages = vec![("reading", started.elapsed())];
        // A deleted meeting's id goes to the next one: nothing more is
        // written once the run is cancelled.
        if operation.is_cancelled() {
            return Ok((stages, CodexVerdict::Unknown));
        }
        if audio.is_empty() {
            self.finish_processing(id, Some(ProcessingError::NoAudio))?;
            return Ok((stages, CodexVerdict::Unknown));
        }
        for (track, audio) in &audio {
            let plan = plan_chunks(&audio.levels.levels, audio.levels.duration_ms);
            self.store.plan_chunks(id, *track, &plan)?;
        }
        if let (Some(mic), Some(system)) = (audio.get(&Track::Mic), audio.get(&Track::System)) {
            // Zeros are what a room meeting records too: they point at the
            // permission only when another app played sound meanwhile.
            let silent = self.store.system_played(id)?
                && system_audio_missing(&mic.levels.levels, &system.levels.levels);
            if silent {
                log::warn!("Meeting {id}: system audio holds only digital silence");
            }
            self.store.set_system_audio_silent(id, silent)?;
        }

        let chunks = self.store.chunks(id)?;
        let jobs: Vec<Job> = chunks
            .iter()
            .filter(|chunk| matches!(chunk.status, ChunkStatus::Pending | ChunkStatus::Failed))
            .filter(|chunk| audio.contains_key(&chunk.track))
            .map(|chunk| Job {
                track: chunk.track,
                idx: chunk.idx,
                start_ms: chunk.start_ms,
                end_ms: chunk.end_ms,
            })
            .collect();
        let saver = Arc::new(Saver {
            id,
            store: Arc::clone(&self.store),
            events: Arc::clone(&self.events),
            done: AtomicU32::new(
                chunks
                    .iter()
                    .filter(|chunk| chunk.status == ChunkStatus::Done)
                    .count() as u32,
            ),
            total: chunks
                .iter()
                .filter(|chunk| chunk.status != ChunkStatus::Silent)
                .count() as u32,
            rate_limited: AtomicBool::new(false),
            unreachable: AtomicBool::new(false),
            answered: AtomicBool::new(false),
        });
        saver.report(false);
        let source = Arc::new(TrackFiles(audio));
        let runner = Runner {
            transcriber: Arc::clone(&self.transcriber),
            // Parakeet would write wrong text in other languages; their
            // pieces wait for Codex instead.
            local: self.local.clone().filter(|_| {
                self.language
                    .as_deref()
                    .is_none_or(crate::engine::parakeet_languages::supports)
            }),
            source: Arc::clone(&source) as Arc<dyn ChunkSource>,
            observer: Arc::clone(&saver) as Arc<dyn RunObserver>,
            policy: self.policy.clone(),
            language: self.language.clone(),
        };
        let by_levels = track_by_levels(&source.0);
        let dir = paths::meeting_dir(&self.root, id);
        let analyzed = AtomicBool::new(false);
        let started = Instant::now();
        let transcribing = async {
            let end = runner.run(jobs, operation.clone()).await;
            if end != RunEnd::Cancelled && !analyzed.load(Ordering::SeqCst) {
                self.report_stage(id, MeetingStage::Analyzing, &saver);
            }
            (end, started.elapsed())
        };
        let analyzing = async {
            let analysis = self
                .analysis(&dir, &source, by_levels, operation.clone())
                .await;
            analyzed.store(true, Ordering::SeqCst);
            (analysis, started.elapsed())
        };
        let ((end, transcribed_in), (analysis, analyzed_in)) =
            tokio::join!(transcribing, analyzing);
        stages.push(("transcription", transcribed_in));
        stages.push(("analysis", analyzed_in));
        let (Some((analysis, outcome)), false) = (analysis, end == RunEnd::Cancelled) else {
            return Ok((stages, saver.verdict()));
        };

        self.report_stage(id, MeetingStage::Aligning, &saver);
        let started = Instant::now();
        let chunks = self.store.chunks(id)?;
        let diarize = analysis
            .as_ref()
            .and_then(|analysis| analysis.diarized)
            .unwrap_or(by_levels);
        let built = build_transcript(
            &chunks,
            &source.0,
            analysis.as_ref(),
            diarize,
            self.language.as_deref(),
            self.store.output_route(id)?,
        );
        // A recorded track that could not be read leaves one side of the
        // conversation out: never a Ready meeting, nor a summary of half of
        // it, nor half a transcript in place of the whole one. Nor does a
        // run that left out more than the transcript it would replace, as
        // "Transcribe again" does while Codex cannot answer.
        let unreadable = recorded_tracks(recorded)
            .iter()
            .any(|track| !source.0.contains_key(track));
        let replaces = meeting.turns.is_empty()
            || (!unreadable && untranscribed_ms(&built.turns) <= untranscribed_ms(&meeting.turns));
        if replaces {
            self.store.save_transcript(
                id,
                &built.speakers,
                &built.turns,
                outcome,
                built.identity.as_deref(),
                built.score,
            )?;
            if !built.turns.is_empty() {
                let dir = paths::meeting_dir(&self.root, id);
                if let Err(error) = super::live::draft::remove(&dir) {
                    log::warn!("Meeting {id}: cannot delete the live transcript: {error}");
                }
            }
        }
        stages.push(("aligning", started.elapsed()));

        let error = if end == RunEnd::SignedOut {
            Some(ProcessingError::SignedOut)
        } else if unreadable {
            Some(ProcessingError::NoAudio)
        } else if !built.missing {
            None
        } else if saver.rate_limited.load(Ordering::SeqCst) {
            Some(ProcessingError::RateLimited)
        } else if end == RunEnd::Unreachable {
            Some(ProcessingError::Offline)
        } else {
            Some(ProcessingError::PartsFailed)
        };
        self.finish_processing(id, error)?;
        log::info!(
            "Meeting {id} processed: {} bubbles, speakers {outcome:?}, alignment {:?}, outcome {error:?}",
            built.turns.len(),
            built.score
        );
        Ok((stages, saver.verdict()))
    }

    fn report_stage(&self, id: i64, stage: MeetingStage, saver: &Saver) {
        if let Err(error) = self.store.set_stage(id, stage) {
            log::error!("Meeting {id}: cannot save the stage: {error}");
        }
        self.events.progress(MeetingProgress {
            id,
            stage,
            chunks_done: saver.done.load(Ordering::SeqCst),
            chunks_total: saver.total,
            rate_limited: false,
        });
    }

    /// The analysis saved by an earlier run, or a new one. `None` when
    /// processing was cancelled.
    async fn analysis(
        &self,
        dir: &Path,
        source: &TrackFiles,
        by_levels: Track,
        operation: ProcessingOperation,
    ) -> Option<(Option<Analysis>, SpeakerAnalysis)> {
        // One saved before its words picked the track whose voices are told
        // apart, and that told them apart on the other, is run again.
        if let Some(saved) = Analysis::load(dir)
            .filter(|saved| saved.diarized == Some(diarized_track(&saved.words, by_levels)))
        {
            let outcome = saved.outcome();
            return Some((Some(saved), outcome));
        }
        let request = AnalysisRequest {
            tracks: source
                .0
                .iter()
                .map(|(track, audio)| (*track, audio.path.clone()))
                .collect(),
            by_levels,
        };
        match self.analyzer.analyze(request, operation).await {
            AnalyzerOutcome::Cancelled => None,
            AnalyzerOutcome::Unavailable(outcome) => Some((None, outcome)),
            AnalyzerOutcome::Ran(analysis) => {
                if let Err(error) = analysis.save(dir) {
                    log::warn!("Cannot save the meeting analysis: {error}");
                }
                let outcome = analysis.outcome();
                Some((Some(analysis), outcome))
            }
        }
    }
}
