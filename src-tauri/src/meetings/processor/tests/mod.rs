mod failures;
mod run;
mod speakers;
mod summary;
mod tracks;

use super::*;
use crate::audio_toolkit::gain::DIGITAL_SILENCE_DBFS;
use crate::cleanup::responses::CompletionError;
use crate::meetings::analysis::ANALYSIS_FILE;
use crate::meetings::capture::route::OutputRoute;
use crate::meetings::chunking;
use crate::meetings::protocol::{Turn, Word};
use crate::meetings::store::{AudioState, ChunkRow, SummaryStatus};
use crate::meetings::summary::SummaryError;
use crate::meetings::track_audio::TrackLevels;
use crate::meetings::transcribe::{Outcome, TranscribeFuture};
use crate::transcription_error::{FailureKind, TranscriptionError};
use crate::OperationId;
use std::collections::BTreeMap;
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
    /// The track its words last picked to tell the speakers apart on.
    diarize: Mutex<Option<Track>>,
}

impl FakeAnalyzer {
    fn answering(outcome: AnalyzerOutcome) -> Self {
        Self {
            answer: Mutex::new(Some(outcome)),
            calls: AtomicU32::new(0),
            diarize: Mutex::default(),
        }
    }

    fn without_models() -> Self {
        Self::answering(AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels))
    }

    /// What the real helper printed for the fixture dialogue.
    fn from_fixture(lines: &str, complete: bool) -> Self {
        Self::from_fixture_on(Track::System, lines, complete)
    }

    /// The fixture dialogue, as if it had been heard on `track`. On the
    /// microphone, nothing is heard on system audio.
    fn from_fixture_on(track: Track, lines: &str, complete: bool) -> Self {
        let mut analysis = Analysis {
            diarized: Some(track),
            ..Analysis::default()
        };
        if track == Track::Mic {
            analysis.words.insert(Track::System, Vec::new());
        }
        let lines = lines.replace(
            r#""track":"system""#,
            &format!(r#""track":"{}""#, track_name(track)),
        );
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
        let mut answer = self
            .answer
            .lock()
            .unwrap()
            .take()
            .unwrap_or(AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels));
        // Like the helper, it tells apart the voices of the track its
        // words pick; scripted turns of another track were never heard.
        if let AnalyzerOutcome::Ran(analysis) = &mut answer {
            let diarize = diarized_track(&analysis.words, request.by_levels);
            if analysis.diarized != Some(diarize) {
                analysis.turns = None;
            }
            analysis.diarized = Some(diarize);
            *self.diarize.lock().unwrap() = Some(diarize);
        }
        Box::pin(async move { answer })
    }
}

/// Codex writing summaries as a script: the next answer each time.
struct FakeSummarizer {
    answers: Mutex<Vec<Result<String, CompletionError>>>,
    inputs: Mutex<Vec<String>>,
    /// What the user does while the first summary is being written.
    meanwhile: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl FakeSummarizer {
    fn answering(answers: Vec<Result<String, CompletionError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers),
            inputs: Mutex::default(),
            meanwhile: Mutex::default(),
        })
    }

    fn calls(&self) -> usize {
        self.inputs.lock().unwrap().len()
    }
}

impl Summarizer for FakeSummarizer {
    fn complete(
        &self,
        _: String,
        input: String,
        _: ProcessingOperation,
    ) -> crate::meetings::summary::CompleteFuture {
        self.inputs.lock().unwrap().push(input);
        if let Some(meanwhile) = self.meanwhile.lock().unwrap().take() {
            meanwhile();
        }
        let mut answers = self.answers.lock().unwrap();
        let answer = if answers.is_empty() {
            Ok("# Réunion\nNotes.".to_string())
        } else {
            answers.remove(0)
        };
        Box::pin(async move { answer })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<MeetingProgress>>, Mutex<Vec<Finished>>);

impl ProcessEvents for Events {
    fn progress(&self, progress: MeetingProgress) {
        self.0.lock().unwrap().push(progress);
    }
    fn updated(&self, _: i64) {}
    fn finished(&self, finished: Finished) {
        self.1.lock().unwrap().push(finished);
    }
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

/// Writes a track of pure digital zeros, what system audio records when
/// nothing plays on the Mac.
fn zeros(path: &Path, seconds: usize) {
    zeros_but(path, seconds, &[]);
}

/// Writes digital zeros except for a loud tone in the given seconds: a
/// notification or a ringtone on a Mac that plays nothing else.
fn zeros_but(path: &Path, seconds: usize, sound: &[(usize, usize)]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for index in 0..seconds * RATE {
        let second = index / RATE;
        let loud = sound
            .iter()
            .any(|(from, to)| (*from..*to).contains(&second));
        let value = if loud {
            ((index as f32 * 0.11).sin() * 3_000.0) as i16
        } else {
            0
        };
        writer.write_sample(value).unwrap();
    }
    writer.finalize().unwrap();
}

fn track_name(track: Track) -> &'static str {
    match track {
        Track::Mic => "mic",
        Track::System => "system",
    }
}

struct Harness {
    store: Arc<MeetingsStore>,
    root: PathBuf,
    events: Arc<Events>,
    /// The language chosen in Settings.
    language: Option<&'static str>,
    /// This Mac's model, when it is ready.
    local: Option<Arc<dyn Transcriber>>,
    /// Off for the tests that record 25 minutes.
    compress_audio: bool,
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
    Harness {
        store: Arc::new(store),
        root: dir.path().join("meetings"),
        events: Arc::default(),
        language: None,
        local: None,
        compress_audio: true,
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

    fn process_with(
        &self,
        id: i64,
        codex: Arc<FakeCodex>,
        analyzer: Arc<FakeAnalyzer>,
    ) -> CodexVerdict {
        self.process_all(id, codex, analyzer, FakeSummarizer::answering(vec![]))
    }

    fn process_all(
        &self,
        id: i64,
        codex: Arc<FakeCodex>,
        analyzer: Arc<FakeAnalyzer>,
        summarizer: Arc<FakeSummarizer>,
    ) -> CodexVerdict {
        let operation = ProcessingOperation::new(OperationId(id as u64));
        self.process_during(id, codex, analyzer, summarizer, operation)
    }

    fn process_during(
        &self,
        id: i64,
        codex: Arc<FakeCodex>,
        analyzer: Arc<FakeAnalyzer>,
        summarizer: Arc<FakeSummarizer>,
        operation: ProcessingOperation,
    ) -> CodexVerdict {
        let processor = Processor {
            store: Arc::clone(&self.store),
            root: self.root.clone(),
            transcriber: codex,
            local: self.local.clone(),
            analyzer,
            summarizer,
            policy: RetryPolicy {
                rate_limit_first: Duration::from_millis(1),
                rate_limit_cap: Duration::from_millis(1),
                rate_limit_budget: Duration::from_millis(3),
                transient: vec![],
            },
            language: self.language.map(str::to_string),
            events: Arc::clone(&self.events) as Arc<dyn ProcessEvents>,
            compress_audio: self.compress_audio,
        };
        tauri::async_runtime::block_on(processor.process(id, operation)).unwrap()
    }
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
const FIXTURE: &str = include_str!("../../fixtures/dialogue_fr.jsonl");

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

fn spoken_meeting(h: &Harness) -> i64 {
    let id = h.meeting(MeetingTracks::Both, 20);
    let dir = paths::meeting_dir(&h.root, id);
    track(&dir.join(paths::SYSTEM_TRACK), 20, &[(1, 6)]);
    track(&dir.join(paths::MIC_TRACK), 20, &[(8, 14)]);
    id
}
