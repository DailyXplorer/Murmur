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
//!    the microphone (`bleed`), and both tracks are interleaved into bubbles
//!    (`merge`). Parts that failed show as placeholders, so the user gets
//!    everything that worked.

use serde::Serialize;
use specta::Type;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use super::align::timing::{sentence_times, time_chunk, words_in};
use super::align::{TimedToken, TimedWord};
use super::analysis::{Analysis, AnalysisRequest, Analyzer, AnalyzerOutcome};
use super::bleed::remove_echoes;
use super::chunking::{plan_chunks, speech_threshold, split_chunk};
use super::merge::{merge, Track, Unit, PLACEHOLDER};
use super::paths;
use super::protocol::{segments, timed_words};
use super::speakers::{self, speaker_row};
use super::spread::{spread, voiced_intervals};
use super::store::{
    ChunkRow, ChunkStatus, MeetingSpeaker, MeetingStage, MeetingTurn, MeetingsStore,
    ProcessingError, SpeakerAnalysis,
};
use super::tokens::{sentences, tokenize};
use super::track_audio::{self, TrackLevels};
use super::transcribe::{
    ChunkSource, Job, Outcome, RetryPolicy, RunEnd, RunObserver, Runner, Transcriber,
};
use crate::transcription_error::FailureKind;
use crate::ProcessingOperation;

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
}

pub struct Processor {
    pub store: Arc<MeetingsStore>,
    pub root: PathBuf,
    pub transcriber: Arc<dyn Transcriber>,
    pub analyzer: Arc<dyn Analyzer>,
    pub policy: RetryPolicy,
    pub language: Option<String>,
    pub events: Arc<dyn ProcessEvents>,
}

fn track_file(track: Track) -> &'static str {
    match track {
        Track::Mic => paths::MIC_TRACK,
        Track::System => paths::SYSTEM_TRACK,
    }
}

/// The key of a track's only speaker when its speakers are not told apart:
/// the microphone is the user, system audio the remote participants.
fn single_speaker(track: Track) -> &'static str {
    match track {
        Track::Mic => "me",
        Track::System => "s1",
    }
}

struct Audio {
    path: PathBuf,
    levels: TrackLevels,
    threshold: Option<f32>,
}

struct TrackFiles(HashMap<Track, Audio>);

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
struct Saver {
    id: i64,
    store: Arc<MeetingsStore>,
    events: Arc<dyn ProcessEvents>,
    done: AtomicU32,
    total: u32,
    rate_limited: AtomicBool,
}

impl Saver {
    fn report(&self, rate_limited: bool) {
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
                self.store.chunk_done(self.id, job.track, job.idx, text)
            }
            Outcome::Failed(kind) => {
                if *kind == FailureKind::RateLimited {
                    self.rate_limited.store(true, Ordering::SeqCst);
                }
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

fn read_tracks(dir: &Path, tracks: &[Track]) -> HashMap<Track, Audio> {
    tracks
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
    /// Processes meeting `id` as far as it can. Does nothing for a meeting
    /// that is gone, still recording or already done.
    pub async fn process(&self, id: i64, operation: ProcessingOperation) -> anyhow::Result<()> {
        if !self.store.begin_processing(id)? {
            return Ok(());
        }
        self.events.updated(id);
        let Some(meeting) = self.store.get(id)? else {
            return Ok(());
        };
        let wanted: Vec<Track> = [
            (meeting.tracks.has_mic(), Track::Mic),
            (meeting.tracks.has_system(), Track::System),
        ]
        .into_iter()
        .filter_map(|(recorded, track)| recorded.then_some(track))
        .collect();
        let dir = paths::meeting_dir(&self.root, id);
        let audio = tokio::task::spawn_blocking(move || read_tracks(&dir, &wanted)).await?;
        if audio.is_empty() {
            self.store
                .finish_processing(id, Some(ProcessingError::NoAudio))?;
            self.events.updated(id);
            return Ok(());
        }
        for (track, audio) in &audio {
            let plan = plan_chunks(&audio.levels.levels, audio.levels.duration_ms);
            self.store.plan_chunks(id, *track, &plan)?;
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
        });
        saver.report(false);
        let source = Arc::new(TrackFiles(audio));
        let runner = Runner {
            transcriber: Arc::clone(&self.transcriber),
            source: Arc::clone(&source) as Arc<dyn ChunkSource>,
            observer: Arc::clone(&saver) as Arc<dyn RunObserver>,
            policy: self.policy.clone(),
            language: self.language.clone(),
        };
        // The remote participants are told apart, or everyone in the room
        // when only the microphone recorded.
        let diarize = if source.0.contains_key(&Track::System) {
            Track::System
        } else {
            Track::Mic
        };
        let dir = paths::meeting_dir(&self.root, id);
        let analyzed = AtomicBool::new(false);
        let transcribing = async {
            let end = runner.run(jobs, operation.clone()).await;
            if end != RunEnd::Cancelled && !analyzed.load(Ordering::SeqCst) {
                self.report_stage(id, MeetingStage::Analyzing, &saver);
            }
            end
        };
        let analyzing = async {
            let analysis = self
                .analysis(&dir, &source, diarize, operation.clone())
                .await;
            analyzed.store(true, Ordering::SeqCst);
            analysis
        };
        let (end, analysis) = tokio::join!(transcribing, analyzing);
        let (Some((analysis, outcome)), false) = (analysis, end == RunEnd::Cancelled) else {
            return Ok(());
        };

        self.report_stage(id, MeetingStage::Aligning, &saver);
        let chunks = self.store.chunks(id)?;
        let built = build_transcript(&chunks, &source.0, analysis.as_ref(), diarize);
        self.store
            .save_transcript(id, &built.speakers, &built.turns, outcome, built.score)?;

        let error = if end == RunEnd::SignedOut {
            Some(ProcessingError::SignedOut)
        } else if !built.missing {
            None
        } else if saver.rate_limited.load(Ordering::SeqCst) {
            Some(ProcessingError::RateLimited)
        } else {
            Some(ProcessingError::PartsFailed)
        };
        self.store.finish_processing(id, error)?;
        self.events.updated(id);
        log::info!(
            "Meeting {id} processed: {} bubbles, speakers {outcome:?}, alignment {:?}, outcome {error:?}",
            built.turns.len(),
            built.score
        );
        Ok(())
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
        diarize: Track,
        operation: ProcessingOperation,
    ) -> Option<(Option<Analysis>, SpeakerAnalysis)> {
        if let Some(saved) = Analysis::load(dir) {
            let outcome = saved.outcome();
            return Some((Some(saved), outcome));
        }
        let request = AnalysisRequest {
            tracks: source
                .0
                .iter()
                .map(|(track, audio)| (*track, audio.path.clone()))
                .collect(),
            diarize,
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

/// The transcript built from the chunks Codex transcribed.
struct Built {
    turns: Vec<MeetingTurn>,
    speakers: Vec<MeetingSpeaker>,
    /// Some parts could not be transcribed.
    missing: bool,
    /// Share of words placed at a measured time, over the chunks that had
    /// on-device words.
    score: Option<f32>,
}

/// Places the words of one transcribed chunk on the track: at the times the
/// on-device recogniser measured, or spread over the track's speech when
/// those cannot be trusted. The last word always ends a sentence, so no
/// sentence runs across chunks.
fn place_chunk(
    text: &str,
    chunk: &ChunkRow,
    audio: &Audio,
    words: Option<&Vec<TimedWord>>,
    score: &mut (f32, usize),
) -> Vec<TimedToken> {
    let timing = words.map(|words| {
        time_chunk(
            tokenize(text),
            words_in(words, chunk.start_ms, chunk.end_ms),
            chunk.start_ms,
            chunk.end_ms,
        )
    });
    let mut tokens = match timing {
        Some(timing) => {
            score.0 += timing.score * timing.comparable as f32;
            score.1 += timing.comparable;
            if timing.trusted() {
                timing.tokens
            } else {
                spread_chunk(text, chunk, audio)
            }
        }
        None => spread_chunk(text, chunk, audio),
    };
    if let Some(last) = tokens.last_mut() {
        last.token.sentence_end = true;
    }
    tokens
}

fn spread_chunk(text: &str, chunk: &ChunkRow, audio: &Audio) -> Vec<TimedToken> {
    let intervals = voiced_intervals(
        &audio.levels.levels,
        audio.threshold,
        chunk.start_ms,
        chunk.end_ms,
    );
    sentence_times(spread(sentences(text), &intervals))
}

fn build_transcript(
    chunks: &[ChunkRow],
    audio: &HashMap<Track, Audio>,
    analysis: Option<&Analysis>,
    diarize: Track,
) -> Built {
    let words: HashMap<Track, Vec<TimedWord>> = analysis
        .map(|analysis| {
            analysis
                .words
                .iter()
                .map(|(track, words)| (*track, timed_words(words)))
                .collect()
        })
        .unwrap_or_default();
    let turns = analysis
        .and_then(|analysis| analysis.turns.as_deref())
        .map(segments);
    let mut units = Vec::new();
    let mut missing = false;
    let mut score = (0.0, 0);
    for track in [Track::System, Track::Mic] {
        let Some(track_audio) = audio.get(&track) else {
            continue;
        };
        let mut stream: Vec<TimedToken> = Vec::new();
        for chunk in chunks.iter().filter(|chunk| chunk.track == track) {
            match (chunk.status, chunk.text.as_deref()) {
                (ChunkStatus::Done, Some(text)) => stream.extend(place_chunk(
                    text,
                    chunk,
                    track_audio,
                    words.get(&track),
                    &mut score,
                )),
                (ChunkStatus::Silent | ChunkStatus::Done, _) => {}
                (ChunkStatus::Pending | ChunkStatus::Failed, _) => {
                    missing = true;
                    units.push(Unit::placeholder(track, chunk.start_ms, chunk.end_ms));
                }
            }
        }
        let diarized = turns
            .as_deref()
            .filter(|_| track == diarize)
            .and_then(|turns| speakers::assign(&stream, turns));
        let keys: Vec<String> = match diarized {
            Some(raw) => {
                let numbers = speakers::number(&raw);
                raw.iter().map(|speaker| numbers[speaker].clone()).collect()
            }
            None => vec![single_speaker(track).to_string(); stream.len()],
        };
        units.extend(speakers::units(track, stream, &keys));
    }
    if let (Some(mic), Some(system), Some(system_words)) = (
        audio.get(&Track::Mic),
        audio.get(&Track::System),
        words.get(&Track::System),
    ) {
        let removed = remove_echoes(
            &mut units,
            system_words,
            &mic.levels.levels,
            &system.levels.levels,
        );
        if removed > 0 {
            log::info!("Removed {removed} microphone sentence(s) echoing system audio");
        }
    }
    let turns: Vec<MeetingTurn> = merge(units)
        .into_iter()
        .map(|bubble| MeetingTurn {
            speaker_key: bubble.speaker,
            start_ms: bubble.start_ms as i64,
            end_ms: bubble.end_ms as i64,
            text: bubble.text,
            low_confidence: bubble.low_confidence,
        })
        .collect();
    let has_me = audio.contains_key(&Track::Mic) && diarize != Track::Mic;
    let keys: BTreeSet<&str> = turns
        .iter()
        .map(|turn| turn.speaker_key.as_str())
        .filter(|key| *key != PLACEHOLDER)
        .collect();
    let mut speakers: Vec<MeetingSpeaker> = keys
        .into_iter()
        .map(|key| speaker_row(key, has_me))
        .collect();
    speakers.sort_by_key(|speaker| speaker.color_index);
    Built {
        speakers,
        missing,
        score: (score.1 > 0).then(|| score.0 / score.1 as f32),
        turns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::store::{MeetingStatus, MeetingTracks};
    use crate::meetings::transcribe::TranscribeFuture;
    use crate::transcription_error::TranscriptionError;
    use crate::OperationId;
    use std::sync::Mutex;
    use std::time::Duration;

    const RATE: usize = 16_000;

    type Answer = Box<dyn Fn(&[f32]) -> Result<String, FailureKind> + Send + Sync>;

    /// Codex as a script that answers from the audio it is sent.
    struct FakeCodex {
        /// The length in seconds of each piece sent.
        calls: Mutex<Vec<usize>>,
        answer: Answer,
    }

    impl Transcriber for FakeCodex {
        fn transcribe(
            &self,
            samples: Vec<f32>,
            _: Option<String>,
            _: ProcessingOperation,
        ) -> TranscribeFuture {
            self.calls.lock().unwrap().push(samples.len() / RATE);
            let result = (self.answer)(&samples)
                .map_err(|kind| TranscriptionError::new(kind, "scripted failure"));
            Box::pin(async move { result })
        }
    }

    fn codex_by_audio(
        answer: impl Fn(&[f32]) -> Result<String, FailureKind> + Send + Sync + 'static,
    ) -> Arc<FakeCodex> {
        Arc::new(FakeCodex {
            calls: Mutex::default(),
            answer: Box::new(answer),
        })
    }

    /// Answers from the length of the piece, in seconds.
    fn codex(
        answer: impl Fn(usize) -> Result<String, FailureKind> + Send + Sync + 'static,
    ) -> Arc<FakeCodex> {
        codex_by_audio(move |samples| answer(samples.len() / RATE))
    }

    /// The on-device analysis as a script.
    struct FakeAnalyzer {
        answer: Mutex<Option<AnalyzerOutcome>>,
        calls: AtomicU32,
    }

    impl FakeAnalyzer {
        fn answering(outcome: AnalyzerOutcome) -> Self {
            Self {
                answer: Mutex::new(Some(outcome)),
                calls: AtomicU32::new(0),
            }
        }

        fn without_models() -> Self {
            Self::answering(AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels))
        }

        /// What the real helper printed for the fixture dialogue.
        fn from_fixture(lines: &str, complete: bool) -> Self {
            let mut analysis = Analysis {
                diarized: Some(Track::System),
                ..Analysis::default()
            };
            for line in lines.lines() {
                analysis.absorb(crate::meetings::protocol::parse_line(line).unwrap());
            }
            analysis.complete = complete;
            Self::answering(AnalyzerOutcome::Ran(analysis))
        }
    }

    impl Analyzer for FakeAnalyzer {
        fn analyze(
            &self,
            request: AnalysisRequest,
            _: ProcessingOperation,
        ) -> crate::meetings::analysis::AnalyzeFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(request.tracks.iter().all(|(_, path)| path.is_file()));
            let answer = self
                .answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or(AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels));
            Box::pin(async move { answer })
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<MeetingProgress>>);

    impl ProcessEvents for Events {
        fn progress(&self, progress: MeetingProgress) {
            self.0.lock().unwrap().push(progress);
        }
        fn updated(&self, _: i64) {}
    }

    /// Writes a track with speech (a loud tone) in the given seconds.
    fn track(path: &Path, seconds: usize, speech: &[(usize, usize)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for index in 0..seconds * RATE {
            let second = index / RATE;
            let loud = speech
                .iter()
                .any(|(from, to)| (*from..*to).contains(&second));
            let value = if loud {
                ((index as f32 * 0.07).sin() * 12_000.0) as i16
            } else {
                ((index * 7919 % 13) as i16) - 6
            };
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }

    struct Harness {
        store: Arc<MeetingsStore>,
        root: PathBuf,
        events: Arc<Events>,
        _dir: tempfile::TempDir,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
        Harness {
            store: Arc::new(store),
            root: dir.path().join("meetings"),
            events: Arc::default(),
            _dir: dir,
        }
    }

    impl Harness {
        fn meeting(&self, tracks: MeetingTracks, seconds: usize) -> i64 {
            let id = self.store.insert_recording(0, tracks).unwrap();
            self.store
                .finish_recording(id, seconds as i64 * 1000, seconds as i64 * 1000)
                .unwrap();
            id
        }

        fn process(&self, id: i64, codex: Arc<FakeCodex>) {
            self.process_with(id, codex, Arc::new(FakeAnalyzer::without_models()));
        }

        fn process_with(&self, id: i64, codex: Arc<FakeCodex>, analyzer: Arc<FakeAnalyzer>) {
            let processor = Processor {
                store: Arc::clone(&self.store),
                root: self.root.clone(),
                transcriber: codex,
                analyzer,
                policy: RetryPolicy {
                    rate_limit_first: Duration::from_millis(1),
                    rate_limit_cap: Duration::from_millis(1),
                    rate_limit_budget: Duration::from_millis(3),
                    transient: vec![],
                },
                language: None,
                events: Arc::clone(&self.events) as Arc<dyn ProcessEvents>,
            };
            tauri::async_runtime::block_on(
                processor.process(id, ProcessingOperation::new(OperationId(id as u64))),
            )
            .unwrap();
        }
    }

    #[test]
    fn both_tracks_become_one_chronological_transcript() {
        let h = harness();
        let id = h.meeting(MeetingTracks::Both, 30);
        let dir = paths::meeting_dir(&h.root, id);
        // The remote participant speaks first, then the user answers.
        track(&dir.join(paths::SYSTEM_TRACK), 30, &[(1, 6), (20, 24)]);
        track(&dir.join(paths::MIC_TRACK), 30, &[(8, 14)]);
        // Each track is one 30 s piece; only system audio speaks at 2 s.
        let codex = codex_by_audio(|samples| {
            let speaks_early = samples[2 * RATE..3 * RATE].iter().any(|s| s.abs() > 0.1);
            Ok(if speaks_early {
                "On commence la réunion maintenant ? Merci.".to_string()
            } else {
                "Oui, allons-y.".to_string()
            })
        });
        h.process(id, Arc::clone(&codex));

        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        assert_eq!(meeting.error, None);
        let turns: Vec<(&str, &str)> = meeting
            .turns
            .iter()
            .map(|turn| (turn.speaker_key.as_str(), turn.text.as_str()))
            .collect();
        assert_eq!(
            turns,
            // The long question spans both stretches of remote speech
            // (1–6 s and 20–24 s), so the answer at 8 s comes before "Merci."
            [
                ("s1", "On commence la réunion maintenant ?"),
                ("me", "Oui, allons-y."),
                ("s1", "Merci."),
            ]
        );
        let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Me", "Speaker 2"]);
        let progress = h.events.0.lock().unwrap();
        let last_transcribing = progress
            .iter()
            .rev()
            .find(|p| p.stage == MeetingStage::Transcribing)
            .unwrap();
        assert_eq!(
            (
                last_transcribing.chunks_done,
                last_transcribing.chunks_total
            ),
            (2, 2)
        );
        assert_eq!(codex.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_failed_part_leaves_a_placeholder_and_a_retry_sends_only_that_part() {
        let h = harness();
        // 25 minutes of speech on one track: three pieces.
        let id = h.meeting(MeetingTracks::Mic, 25 * 60);
        let dir = paths::meeting_dir(&h.root, id);
        track(&dir.join(paths::MIC_TRACK), 25 * 60, &[(0, 25 * 60)]);
        // Exactly one piece is refused.
        let refuse = Arc::new(AtomicBool::new(true));
        let refusing = Arc::clone(&refuse);
        let first = codex(move |seconds| {
            if refusing.swap(false, Ordering::SeqCst) {
                Err(FailureKind::Rejected)
            } else {
                Ok(format!("Partie de {seconds} secondes."))
            }
        });
        h.process(id, Arc::clone(&first));

        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Failed);
        assert_eq!(meeting.error, Some(ProcessingError::PartsFailed));
        assert_eq!(first.calls.lock().unwrap().len(), 3);
        let placeholders: Vec<_> = meeting
            .turns
            .iter()
            .filter(|turn| turn.speaker_key == crate::meetings::merge::PLACEHOLDER)
            .collect();
        assert_eq!(placeholders.len(), 1);
        // Every piece lasts at least five minutes.
        assert!(placeholders[0].end_ms - placeholders[0].start_ms >= 5 * 60 * 1000);

        // A restart or Retry sends only the piece that failed.
        let second = codex(|seconds| Ok(format!("Partie de {seconds} secondes.")));
        h.process(id, Arc::clone(&second));
        assert_eq!(second.calls.lock().unwrap().len(), 1);
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        assert_eq!(meeting.turns.len(), 1);
        assert_eq!(meeting.turns[0].speaker_key, "me");
        assert_eq!(meeting.turns[0].text.matches("Partie").count(), 3);
    }

    #[test]
    fn pieces_already_transcribed_are_not_sent_again_after_a_crash() {
        let h = harness();
        let id = h.meeting(MeetingTracks::Mic, 25 * 60);
        let dir = paths::meeting_dir(&h.root, id);
        track(&dir.join(paths::MIC_TRACK), 25 * 60, &[(0, 25 * 60)]);
        // A first run planned the pieces and saved the first one, then the
        // app was killed with the meeting still processing.
        let levels = track_audio::levels(&dir.join(paths::MIC_TRACK)).unwrap();
        h.store
            .plan_chunks(
                id,
                Track::Mic,
                &plan_chunks(&levels.levels, levels.duration_ms),
            )
            .unwrap();
        h.store.chunk_done(id, Track::Mic, 0, "Déjà fait.").unwrap();

        let codex = codex(|_| Ok("Nouveau.".to_string()));
        h.process(id, Arc::clone(&codex));

        assert_eq!(codex.calls.lock().unwrap().len(), 2);
        let meeting = h.store.get(id).unwrap().unwrap();
        assert!(meeting.turns[0].text.starts_with("Déjà fait."));
        assert_eq!(meeting.status, MeetingStatus::Ready);
    }

    #[test]
    fn silent_pieces_are_never_sent() {
        let h = harness();
        let id = h.meeting(MeetingTracks::Both, 40);
        let dir = paths::meeting_dir(&h.root, id);
        track(&dir.join(paths::MIC_TRACK), 40, &[(2, 10)]);
        track(&dir.join(paths::SYSTEM_TRACK), 40, &[]);
        let codex = codex(|_| Ok("Seul.".to_string()));
        h.process(id, Arc::clone(&codex));
        assert_eq!(codex.calls.lock().unwrap().len(), 1);
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        assert_eq!(meeting.turns.len(), 1);
    }

    #[test]
    fn a_lost_session_fails_the_meeting_with_sign_in_needed() {
        let h = harness();
        let id = h.meeting(MeetingTracks::Mic, 20);
        track(
            &paths::meeting_dir(&h.root, id).join(paths::MIC_TRACK),
            20,
            &[(0, 20)],
        );
        h.process(id, codex(|_| Err(FailureKind::SessionExpired)));
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Failed);
        assert_eq!(meeting.error, Some(ProcessingError::SignedOut));
    }

    #[test]
    fn a_meeting_without_readable_audio_fails_and_a_ready_one_is_left_alone() {
        let h = harness();
        let id = h.meeting(MeetingTracks::Both, 20);
        let codex = codex(|_| Ok("x".into()));
        h.process(id, Arc::clone(&codex));
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.error, Some(ProcessingError::NoAudio));

        let ready = h.meeting(MeetingTracks::Mic, 20);
        h.store.begin_processing(ready).unwrap();
        h.store.finish_processing(ready, None).unwrap();
        h.process(ready, Arc::clone(&codex));
        assert!(codex.calls.lock().unwrap().is_empty());
    }

    /// A fictional dialogue between two `say` voices; `fixtures/dialogue_fr.jsonl`
    /// is what the helper printed for it. The second voice is poorly heard
    /// on device, so many of its words are not anchors.
    const DIALOGUE: [&str; 8] = [
        "Bonjour à tous, on commence la réunion de suivi du projet.",
        "Bonjour. Oui, j'ai préparé le point sur le budget.",
        "Parfait. Où en est la plateforme de facturation ?",
        "La migration est terminée, il reste les tests de charge.",
        "D'accord. On peut livrer jeudi prochain ?",
        "Oui, si l'équipe client valide les scénarios avant mercredi.",
        "Très bien, je m'occupe de les relancer aujourd'hui.",
        "Merci. Dernier point, le séminaire est décalé en mars.",
    ];
    const FIXTURE: &str = include_str!("fixtures/dialogue_fr.jsonl");

    fn dialogue_meeting(h: &Harness) -> i64 {
        let id = h.meeting(MeetingTracks::System, 34);
        track(
            &paths::meeting_dir(&h.root, id).join(paths::SYSTEM_TRACK),
            34,
            &[(0, 34)],
        );
        id
    }

    fn turns(meeting: &crate::meetings::store::MeetingDetail) -> Vec<(&str, &str)> {
        meeting
            .turns
            .iter()
            .map(|turn| (turn.speaker_key.as_str(), turn.text.as_str()))
            .collect()
    }

    #[test]
    fn each_sentence_goes_to_the_speaker_who_said_it() {
        let h = harness();
        let id = dialogue_meeting(&h);
        let analyzer = Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true));
        h.process_with(id, codex(|_| Ok(DIALOGUE.join(" "))), Arc::clone(&analyzer));

        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
        assert_eq!(
            turns(&meeting),
            [
                ("s1", DIALOGUE[0]),
                ("s2", DIALOGUE[1]),
                ("s1", DIALOGUE[2]),
                ("s2", DIALOGUE[3]),
                ("s1", DIALOGUE[4]),
                ("s2", DIALOGUE[5]),
                // The diarizer heard nobody during "Merci.", and the turn
                // that ended closest is the other voice's.
                (
                    "s1",
                    "Très bien, je m'occupe de les relancer aujourd'hui. Merci."
                ),
                ("s2", "Dernier point, le séminaire est décalé en mars."),
            ]
        );
        let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Speaker 1", "Speaker 2"]);
        assert!(h
            .events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.stage == MeetingStage::Aligning));

        // The saved analysis is reused: rebuilding runs nothing again.
        assert!(h.store.reopen(id).unwrap());
        let again = Arc::new(FakeAnalyzer::without_models());
        let silent = codex(|_| Ok("never".to_string()));
        h.process_with(id, Arc::clone(&silent), Arc::clone(&again));
        assert_eq!(again.calls.load(Ordering::SeqCst), 0);
        assert!(silent.calls.lock().unwrap().is_empty());
        assert_eq!(turns(&h.store.get(id).unwrap().unwrap()).len(), 8);
    }

    #[test]
    fn a_crashed_analysis_keeps_every_word_under_one_speaker_and_runs_again() {
        let h = harness();
        let id = dialogue_meeting(&h);
        // The helper printed the words, then died before the diarization.
        let words_only = FIXTURE.lines().next().unwrap();
        h.process_with(
            id,
            codex(|_| Ok(DIALOGUE.join(" "))),
            Arc::new(FakeAnalyzer::from_fixture(words_only, false)),
        );
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.status, MeetingStatus::Ready);
        assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Failed));
        assert!(meeting.turns.iter().all(|turn| turn.speaker_key == "s1"));
        let text: Vec<&str> = meeting
            .turns
            .iter()
            .map(|turn| turn.text.as_str())
            .collect();
        assert_eq!(text.join(" "), DIALOGUE.join(" "));

        // Re-analysing runs the helper again, since it did not finish.
        assert!(h.store.reopen(id).unwrap());
        let full = Arc::new(FakeAnalyzer::from_fixture(FIXTURE, true));
        h.process_with(id, codex(|_| Ok("never".to_string())), Arc::clone(&full));
        assert_eq!(full.calls.load(Ordering::SeqCst), 1);
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::Separated));
        assert_eq!(meeting.speakers.len(), 2);
    }

    #[test]
    fn without_models_each_track_has_one_speaker() {
        let h = harness();
        let id = dialogue_meeting(&h);
        h.process(id, codex(|_| Ok(DIALOGUE.join(" "))));
        let meeting = h.store.get(id).unwrap().unwrap();
        assert_eq!(meeting.speaker_analysis, Some(SpeakerAnalysis::NoModels));
        let names: Vec<&str> = meeting.speakers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Speaker 1"]);
    }
}
