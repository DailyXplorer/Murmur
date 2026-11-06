//! Windows: the words of a meeting with their times, and who spoke when, from
//! models on the CPU inside the app: Parakeet Ultra (transcribe.cpp) for the
//! words, pyannote Community-1 (ONNX Runtime, `diarize/`) for the voices, any
//! number of them. There is no helper process here, so a crash in native code
//! is answered with `catch_unwind`, a time limit scaled to the audio, and a
//! cancel that stops the models between their steps. What decides the result
//! is in `analysis_local`, which a test runs without models.

use anyhow::{anyhow, bail, Context};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transcribe_cpp::{
    CancelToken, Error, ErrorKind, Model, RunOptions, Session, SessionOptions, TimestampKind,
};

use super::analysis::{Analysis, AnalysisRequest, AnalyzeFuture, Analyzer, AnalyzerOutcome};
use super::analysis_local::{self as local, Diarizer, Failure, Heard, Recognizer, Run, TrackAudio};
use super::diarize::pyannote;
use super::helper::Ended;
use super::merge::Track;
use super::models::Models;
use super::protocol::{ErrorCode, Message, Phase};
use super::store::SpeakerAnalysis;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::pauses::{is_silent, speech_pieces};
use crate::engine::local::cpu::load_model;
use crate::engine::local::MAX_PIECE;
use crate::ProcessingOperation;

/// How long a stopped run is given to return before it is left behind.
const STOP_GRACE: Duration = Duration::from_secs(30);

/// Runs the analysis in the app with the downloaded models.
pub struct CppAnalyzer {
    pub models: Arc<Models>,
}

impl Analyzer for CppAnalyzer {
    fn analyze(&self, request: AnalysisRequest, operation: ProcessingOperation) -> AnalyzeFuture {
        let models = Arc::clone(&self.models);
        Box::pin(async move {
            // A download started with the meeting may still be finishing.
            if !models.wait_ready(&operation).await {
                return if operation.is_cancelled() {
                    AnalyzerOutcome::Cancelled
                } else {
                    AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels)
                };
            }
            // The speaker model comes after the speech model. Without it
            // the words are timed all the same.
            let speaker_model = models.wait_optional(&operation).await;
            if !speaker_model && operation.is_cancelled() {
                return AnalyzerOutcome::Cancelled;
            }
            let started = Instant::now();
            // The model looked for, as the helper analyzer reads it.
            let asr = crate::local_models::asr_in_use(models.dir());
            let track_ms: Vec<u64> = request
                .tracks
                .iter()
                .map(|(_, path)| duration_ms(path))
                .collect();
            let limit = local::time_limit(&track_ms);
            let stop = CancelToken::new();
            let dir = models.dir().to_path_buf();
            let mut job = tokio::task::spawn_blocking({
                let stop = stop.clone();
                move || analyze_blocking(&dir, &request, speaker_model, &stop)
            });
            let deadline = tokio::time::sleep(limit);
            tokio::pin!(deadline);
            let mut stopped: Option<Ended> = None;
            let mut joined = None;
            tokio::select! {
                result = &mut job => joined = Some(result),
                () = operation.cancelled() => stopped = Some(Ended::Cancelled),
                () = &mut deadline => stopped = Some(Ended::TimedOut),
            }
            if joined.is_none() {
                // The models stop between their steps. One that does not is
                // left behind rather than waited for.
                stop.cancel();
                joined = tokio::time::timeout(STOP_GRACE, &mut job).await.ok();
            }
            let mut finished = match joined {
                Some(Ok(finished)) => finished,
                Some(Err(_)) => {
                    log::error!("The meeting analysis thread ended without an answer");
                    Finished::crashed(Analysis::default())
                }
                None => {
                    log::error!("The meeting analysis did not stop in time");
                    Finished::crashed(Analysis::default())
                }
            };
            // A run past its time limit was stopped like a cancelled one.
            match stopped {
                Some(Ended::Cancelled) => return AnalyzerOutcome::Cancelled,
                Some(ended) => finished.ended = ended,
                None => {}
            }
            let Finished {
                mut analysis,
                ended,
                speech_model_missing,
            } = finished;
            match ended {
                Ended::Done => analysis.complete = true,
                Ended::Cancelled => return AnalyzerOutcome::Cancelled,
                ended => log::warn!("Meeting analysis ended early: {ended:?}"),
            }
            if analysis.models_missing {
                // Run again once the models are back.
                analysis.complete = false;
                if speech_model_missing {
                    models.invalidate(asr);
                }
            }
            log::info!(
                "Meeting analysis: {:?} in {:.1} s",
                analysis.outcome(),
                started.elapsed().as_secs_f32()
            );
            AnalyzerOutcome::Ran(analysis)
        })
    }
}

/// The length of a track from its header, 0 when it cannot be read.
fn duration_ms(path: &Path) -> u64 {
    hound::WavReader::open(path)
        .map(|reader| u64::from(reader.duration()) * 1000 / u64::from(TARGET_SAMPLE_RATE))
        .unwrap_or(0)
}

struct Finished {
    analysis: Analysis,
    ended: Ended,
    /// The speech model could not be opened: the models are downloaded again.
    speech_model_missing: bool,
}

impl Finished {
    fn crashed(analysis: Analysis) -> Self {
        Self {
            analysis,
            ended: Ended::Crashed,
            speech_model_missing: false,
        }
    }
}

/// How many CPU threads the models may use: half of them, so a dictation
/// made while a meeting is analysed still finds a processor to run on.
fn threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(2, usize::from);
    i32::try_from((cores / 2).max(1)).unwrap_or(1)
}

/// The analysis on the calling thread. A panic in the models ends it as a
/// crash and keeps what was found before.
fn analyze_blocking(
    dir: &Path,
    request: &AnalysisRequest,
    speaker_model: bool,
    stop: &CancelToken,
) -> Finished {
    let mut analysis = Analysis::default();
    let mut speech_model_missing = false;
    let ended = catch_unwind(AssertUnwindSafe(|| {
        run_models(
            dir,
            request,
            speaker_model,
            stop,
            &mut analysis,
            &mut speech_model_missing,
        )
    }))
    .unwrap_or_else(|_| {
        log::error!("The meeting analysis stopped on a panic in the models");
        Ended::Crashed
    });
    Finished {
        analysis,
        ended,
        speech_model_missing,
    }
}

fn run_models(
    dir: &Path,
    request: &AnalysisRequest,
    speaker_model: bool,
    stop: &CancelToken,
    analysis: &mut Analysis,
    speech_model_missing: &mut bool,
) -> Ended {
    let model = match load_model(dir) {
        Ok(model) => model,
        Err(error) => {
            let kind = error.downcast_ref::<Error>().map(Error::kind);
            log::error!("Meeting analysis: cannot load the speech model ({kind:?})");
            *speech_model_missing = kind == Some(ErrorKind::ModelFileNotFound);
            analysis.absorb(Message::Error {
                stage: "asr".to_string(),
                code: if *speech_model_missing {
                    ErrorCode::ModelMissing
                } else {
                    ErrorCode::Crash
                },
                message: String::new(),
            });
            return Ended::Crashed;
        }
    };
    let mut recognizer = match Parakeet::new(&model, stop) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            log::error!(
                "Meeting analysis: cannot open the speech model ({:?})",
                error.kind()
            );
            analysis.absorb(Message::Error {
                stage: "asr".to_string(),
                code: ErrorCode::Crash,
                message: String::new(),
            });
            return Ended::Crashed;
        }
    };
    let is_stopped = || stop.is_cancelled();
    let mut speakers: Option<Box<dyn Diarizer + '_>> = if speaker_model {
        match pyannote::load(dir, usize::try_from(threads()).unwrap_or(1), &is_stopped) {
            Ok(speakers) => Some(Box::new(speakers)),
            // Missing or damaged: the words are timed, one speaker per track.
            Err(Failure::Missing) => {
                log::warn!("Meeting analysis: no speaker model");
                None
            }
            // The files are there but cannot run (a processor without AVX2, a
            // runtime that does not start): the same, and the run says so.
            Err(failure) => {
                log::warn!("Meeting analysis: the speaker model cannot run ({failure:?})");
                Some(Box::new(Unusable(failure)))
            }
        }
    } else {
        None
    };
    let mut last_step: Option<(Phase, Track, u32)> = None;
    local::run(
        &Run {
            tracks: &request.tracks,
            by_levels: request.by_levels,
            audio: &Disk,
            stop: &|| stop.is_cancelled(),
        },
        &mut recognizer,
        speakers.as_deref_mut(),
        analysis,
        &mut |phase, track, fraction| {
            // A line per tenth of a phase: numbers and fixed words only.
            let tenth = (fraction * 10.0) as u32;
            if last_step != Some((phase, track, tenth)) {
                last_step = Some((phase, track, tenth));
                log::debug!("Meeting analysis: {phase:?} {track:?} {}%", tenth * 10);
            }
        },
    )
}

/// How a native error ends a model call.
fn failure(error: &Error) -> Failure {
    match error.kind() {
        ErrorKind::Aborted => Failure::Stopped,
        ErrorKind::ModelFileNotFound => Failure::Missing,
        kind => Failure::Failed(format!("{kind:?}")),
    }
}

/// Parakeet Ultra, asked for the time of every word.
struct Parakeet {
    session: Session,
}

impl Parakeet {
    fn new(model: &Model, stop: &CancelToken) -> transcribe_cpp::Result<Self> {
        let mut session = model.session_with(&SessionOptions {
            n_threads: threads(),
            ..SessionOptions::default()
        })?;
        session.set_cancel_token(stop);
        Ok(Self { session })
    }
}

impl Recognizer for Parakeet {
    fn hear(&mut self, piece: &[f32]) -> Result<Vec<Heard>, Failure> {
        let options = RunOptions {
            timestamps: TimestampKind::Word,
            ..RunOptions::default()
        };
        let transcript = match self.session.run(piece, &options) {
            Ok(transcript) => transcript,
            Err(error) => match (error.kind(), error.partial()) {
                // A decode that stopped early keeps the words it found.
                (ErrorKind::OutputTruncated | ErrorKind::OutputRepetition, Some(partial)) => {
                    partial.clone()
                }
                _ => return Err(failure(&error)),
            },
        };
        Ok(transcript
            .words
            .into_iter()
            .map(|word| Heard {
                text: word.text,
                start_ms: word.t0_ms,
                end_ms: word.t1_ms,
            })
            .collect())
    }
}

/// A speaker model that cannot run: the words are timed, the run ends as a
/// failure of the speaker stage and each track keeps one speaker.
struct Unusable(Failure);

impl Diarizer for Unusable {
    fn tell_apart(&mut self, _: &[f32]) -> Result<Vec<local::Spoken>, Failure> {
        Err(self.0.clone())
    }
}

/// Meeting tracks: 16 kHz mono 16-bit WAV files, read whole into memory
/// (about 700 MB of samples for the longest meeting, a quarter of that
/// per hour), cut at pauses into pieces the speech model takes.
struct Disk;

impl TrackAudio for Disk {
    fn read(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
        read_track(path)
    }

    fn pieces(&self, samples: &[f32]) -> Vec<Range<usize>> {
        speech_pieces(samples, MAX_PIECE)
    }

    fn is_silent(&self, samples: &[f32]) -> bool {
        is_silent(samples)
    }
}

fn read_track(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("Cannot open {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != TARGET_SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        bail!("{} is not a 16 kHz mono track", path.display());
    }
    // A track too long for the memory left is an error, not an abort.
    let mut samples: Vec<f32> = Vec::new();
    samples
        .try_reserve_exact(reader.duration() as usize)
        .map_err(|_| anyhow!("Not enough memory to read {}", path.display()))?;
    for sample in reader.samples::<i16>() {
        samples.push(f32::from(sample?) / f32::from(i16::MAX));
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_models::manifest::{pyannote_path, PYANNOTE_FILES};
    use crate::meetings::protocol::timed_words;
    use std::path::PathBuf;

    fn write(path: &Path, spec: hound::WavSpec, samples: &[i16]) {
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn track_spec() -> hound::WavSpec {
        hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        }
    }

    #[test]
    fn a_track_is_read_whole_and_its_length_comes_from_the_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        write(&path, track_spec(), &[0, i16::MAX, i16::MIN + 1, 0]);
        let samples = read_track(&path).unwrap();
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[1], 1.0);
        assert_eq!(samples[2], -1.0);

        let long = dir.path().join("long.wav");
        write(&long, track_spec(), &[0i16; 16_000 * 3]);
        assert_eq!(duration_ms(&long), 3_000);
        assert_eq!(duration_ms(&dir.path().join("missing.wav")), 0);
    }

    #[test]
    fn a_file_that_is_not_a_meeting_track_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let stereo = dir.path().join("stereo.wav");
        write(
            &stereo,
            hound::WavSpec {
                channels: 2,
                ..track_spec()
            },
            &[0; 8],
        );
        assert!(read_track(&stereo).is_err());
        let fast = dir.path().join("fast.wav");
        write(
            &fast,
            hound::WavSpec {
                sample_rate: 48_000,
                ..track_spec()
            },
            &[0; 8],
        );
        assert!(read_track(&fast).is_err());
        assert!(read_track(&dir.path().join("missing.wav")).is_err());
    }

    #[test]
    fn the_threads_leave_half_of_the_processor_to_a_dictation() {
        let cores = std::thread::available_parallelism().map_or(2, usize::from);
        let used = usize::try_from(threads()).unwrap();
        assert!((1..=cores.max(1)).contains(&used));
        assert_eq!(used, (cores / 2).max(1));
    }

    /// Loads the CPU modules, which sit beside the app: CI runs it with the
    /// models (`cpu model` on Windows).
    #[test]
    #[ignore = "loads the CPU modules of transcribe.cpp"]
    fn a_folder_without_the_speech_model_ends_the_analysis_without_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let request = AnalysisRequest {
            tracks: vec![(Track::Mic, dir.path().join("mic.wav"))],
            by_levels: Track::Mic,
        };
        let finished = analyze_blocking(dir.path(), &request, false, &CancelToken::new());
        assert_eq!(finished.ended, Ended::Crashed);
        assert!(finished.analysis.words.is_empty());
        assert!(matches!(
            finished.analysis.outcome(),
            SpeakerAnalysis::NoModels | SpeakerAnalysis::Failed
        ));
    }

    /// Runs the analysis on the dialogue of `clips` with the models of
    /// `MURMUR_SMOKE_MODELS`. The folder holds `u1.wav`, `u2.wav`, `u3.wav`
    /// (16 kHz mono, spoken in turn by two voices) with the words of each in
    /// `u<N>.txt`, one per line, and `one-voice` when the machine had only
    /// one voice.
    fn check_dialogue(clips: &Path) {
        let models =
            PathBuf::from(std::env::var("MURMUR_SMOKE_MODELS").expect("MURMUR_SMOKE_MODELS"));
        for name in PYANNOTE_FILES {
            assert!(
                models.join(pyannote_path(name)).is_file(),
                "{name} is not fetched"
            );
        }

        // The dialogue: each utterance, then a second of silence.
        let gap = TARGET_SAMPLE_RATE as usize;
        let mut dialogue: Vec<f32> = vec![0.0; gap];
        let mut spans: Vec<(f64, f64)> = Vec::new();
        let mut expected: Vec<String> = Vec::new();
        for number in 1..=3 {
            let utterance = read_track(&clips.join(format!("u{number}.wav"))).unwrap();
            let start = dialogue.len();
            dialogue.extend_from_slice(&utterance);
            spans.push((
                start as f64 / f64::from(TARGET_SAMPLE_RATE),
                dialogue.len() as f64 / f64::from(TARGET_SAMPLE_RATE),
            ));
            dialogue.resize(dialogue.len() + gap, 0.0);
            let words = std::fs::read_to_string(clips.join(format!("u{number}.txt"))).unwrap();
            expected.extend(
                words
                    .lines()
                    .map(|word| word.trim().to_lowercase())
                    .filter(|word| !word.is_empty()),
            );
        }
        let length = dialogue.len() as f64 / f64::from(TARGET_SAMPLE_RATE);
        let work = tempfile::tempdir().unwrap();
        let track = work.path().join("system.wav");
        let pcm: Vec<i16> = dialogue
            .iter()
            .map(|sample| (sample * f32::from(i16::MAX)) as i16)
            .collect();
        write(&track, track_spec(), &pcm);

        let request = AnalysisRequest {
            tracks: vec![(Track::System, track)],
            by_levels: Track::System,
        };
        let started = Instant::now();
        let finished = analyze_blocking(&models, &request, true, &CancelToken::new());
        println!(
            "analysed {length:.1} s of speech in {:.1} s: {:?}",
            started.elapsed().as_secs_f32(),
            finished.analysis.outcome()
        );
        assert_eq!(finished.ended, Ended::Done);
        let analysis = finished.analysis;
        assert_eq!(analysis.diarized, Some(Track::System));

        // Word times: in order, inside the clip, for most of the words said.
        let words = &analysis.words[&Track::System];
        assert!(words.windows(2).all(|pair| pair[0].s <= pair[1].s));
        for word in words {
            assert!(
                word.s >= 0.0 && word.s <= word.e && word.e <= length,
                "{word:?}"
            );
        }
        let heard: Vec<String> = timed_words(words)
            .into_iter()
            .map(|word| word.key)
            .collect();
        let timed = expected
            .iter()
            .filter(|word| heard.contains(&crate::meetings::tokens::key(word)))
            .count();
        println!("{timed} of {} expected words timed", expected.len());
        assert!(
            timed * 10 >= expected.len() * 8,
            "{timed} of {}",
            expected.len()
        );

        // Two voices: told apart, with the turn changing near each splice.
        if clips.join("one-voice").exists() {
            println!("one voice on this machine: speakers not checked");
            return;
        }
        let turns = analysis.turns.as_ref().expect("no turns");
        let overlap = |turn: &crate::meetings::protocol::Turn, from: f64, to: f64| {
            (turn.e.min(to) - turn.s.max(from)).max(0.0)
        };
        let owner = |from: f64, to: f64| {
            let mut by_speaker: std::collections::BTreeMap<&str, f64> = Default::default();
            for turn in turns {
                *by_speaker.entry(turn.spk.as_str()).or_default() += overlap(turn, from, to);
            }
            by_speaker
                .into_iter()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(speaker, _)| speaker.to_string())
                .unwrap()
        };
        let speakers: std::collections::BTreeSet<&str> =
            turns.iter().map(|turn| turn.spk.as_str()).collect();
        println!("speakers: {speakers:?}, {} turns", turns.len());
        assert!(speakers.len() >= 2, "{speakers:?}");
        let first = owner(spans[0].0, spans[0].1);
        let second = owner(spans[1].0, spans[1].1);
        assert_ne!(first, second, "the two voices got one speaker");
        // The change of speaker at the splice: the second voice's turn
        // starts near the second utterance, the first voice's ends near the
        // end of the first.
        let near = |a: f64, b: f64| (a - b).abs() <= 1.5;
        let second_starts = turns
            .iter()
            .filter(|turn| turn.spk == second && overlap(turn, spans[1].0, spans[1].1) > 0.0)
            .map(|turn| turn.s)
            .fold(f64::INFINITY, f64::min);
        let first_ends = turns
            .iter()
            .filter(|turn| turn.spk == first && overlap(turn, spans[0].0, spans[0].1) > 0.0)
            .map(|turn| turn.e)
            .fold(f64::NEG_INFINITY, f64::max);
        println!(
            "splice at {:.1} s: second voice from {second_starts:.1} s, first ends {first_ends:.1} s",
            spans[1].0
        );
        assert!(near(second_starts, spans[1].0), "{second_starts}");
        assert!(near(first_ends, spans[0].1), "{first_ends}");
    }

    /// Needs the models and the spoken utterances, so CI runs it on its own
    /// (`cpu model` on Windows): `MURMUR_SMOKE_MODELS=<models dir>
    /// MURMUR_SMOKE_DIALOGUE=<dir> cargo test --lib
    /// meetings::analysis_cpp -- --ignored`.
    #[test]
    #[ignore = "needs the models and recorded speech"]
    fn two_voices_are_timed_and_told_apart() {
        let clips =
            PathBuf::from(std::env::var("MURMUR_SMOKE_DIALOGUE").expect("MURMUR_SMOKE_DIALOGUE"));
        check_dialogue(&clips);
    }

    /// The same in French, where the runner has French voices; it says so and
    /// passes without checking anything where it has none.
    #[test]
    #[ignore = "needs the models and recorded French speech"]
    fn two_french_voices_are_timed_and_told_apart() {
        let clips = PathBuf::from(
            std::env::var("MURMUR_SMOKE_DIALOGUE_FR").expect("MURMUR_SMOKE_DIALOGUE_FR"),
        );
        if !clips.join("u1.wav").is_file() {
            println!("no French voice on this machine: the French dialogue is skipped");
            return;
        }
        check_dialogue(&clips);
    }
}
