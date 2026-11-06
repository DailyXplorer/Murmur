//! The on-device analysis of a meeting when it runs inside the app (Windows):
//! the same result as `murmur-meetings-helper`, a `protocol` message per
//! track, so `Analysis::absorb`, `align/`, `speakers.rs` and `bleed.rs` are
//! unchanged. The speech and speaker models sit behind two traits, so what
//! decides the result (pieces, times, which track is told apart, how a
//! failure ends) is tested without a model. `analysis_cpp` holds the models.

use std::borrow::Cow;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::analysis::{diarized_track, Analysis};
use super::helper::Ended;
use super::merge::Track;
use super::protocol::{ErrorCode, Message, Phase, Turn, Word};

pub const SAMPLES_PER_SECOND: usize = 16_000;
/// A shorter piece gets silence after it, as the dictation engine does.
const MIN_PIECE: usize = SAMPLES_PER_SECOND / 2;
/// The time limit of a run before it counts any audio: loading the models.
const LIMIT_BASE: Duration = Duration::from_secs(10 * 60);
/// A run may take this many times the audio it reads. A weak processor
/// recognises speech slower than real time; this only catches a hung model.
const LIMIT_PER_AUDIO: u32 = 2;

/// A word as the recogniser reports it. Times are milliseconds from the start
/// of the piece it heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Who spoke over a stretch of a track: milliseconds on the track and the
/// model's 1-based speaker number.
#[derive(Debug, Clone, PartialEq)]
pub struct Spoken {
    pub speaker: i32,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Why a model did not answer. The text is a fixed word, never audio or
/// transcript content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The model file is not there (or no longer): download it again.
    Missing,
    /// Cancelled, or past its time limit.
    Stopped,
    Failed(String),
}

pub trait Recognizer {
    /// The words of one piece of 16 kHz mono audio, at least half a second.
    fn hear(&mut self, piece: &[f32]) -> Result<Vec<Heard>, Failure>;
}

pub trait Diarizer {
    /// Who spoke when on a whole track of 16 kHz mono audio.
    fn tell_apart(&mut self, track: &[f32]) -> Result<Vec<Spoken>, Failure>;

    /// `tell_apart`, reporting the share of the track done (0.0 to 1.0) as it
    /// goes. A model that cannot say reports nothing.
    fn tell_apart_reporting(
        &mut self,
        track: &[f32],
        progress: &mut dyn FnMut(f64),
    ) -> Result<Vec<Spoken>, Failure> {
        let _ = progress;
        self.tell_apart(track)
    }
}

/// The audio side of a run: reading a track, cutting it into pieces the
/// recogniser takes, and telling silence from speech.
pub trait TrackAudio {
    fn read(&self, path: &Path) -> anyhow::Result<Vec<f32>>;
    /// The pieces with speech, in order; none for silence.
    fn pieces(&self, samples: &[f32]) -> Vec<Range<usize>>;
    fn is_silent(&self, samples: &[f32]) -> bool;
}

pub struct Run<'a> {
    pub tracks: &'a [(Track, PathBuf)],
    /// Which track to tell apart when the words cannot say.
    pub by_levels: Track,
    pub audio: &'a dyn TrackAudio,
    /// Asked between pieces: a run that is stopped ends as `Cancelled`.
    pub stop: &'a dyn Fn() -> bool,
}

/// The longest a run on tracks of these lengths may take.
pub fn time_limit(track_ms: &[u64]) -> Duration {
    let all: u64 = track_ms.iter().sum();
    // One track is read again to tell its voices apart.
    let again = track_ms.iter().copied().max().unwrap_or(0);
    LIMIT_BASE + Duration::from_millis(all + again) * LIMIT_PER_AUDIO
}

fn padded(piece: &[f32]) -> Cow<'_, [f32]> {
    if piece.len() >= MIN_PIECE {
        return Cow::Borrowed(piece);
    }
    let mut padded = piece.to_vec();
    padded.resize(MIN_PIECE, 0.0);
    Cow::Owned(padded)
}

fn samples_ms(samples: usize) -> u64 {
    (samples * 1000 / SAMPLES_PER_SECOND) as u64
}

/// The words of a piece that starts `offset_ms` into its track, in seconds on
/// the track. A word the model put outside the piece is brought inside it.
pub fn words_at(heard: Vec<Heard>, offset_ms: u64, piece_ms: u64) -> Vec<Word> {
    let offset = offset_ms as i64;
    let length = piece_ms as i64;
    heard
        .into_iter()
        .filter_map(|word| {
            let text = word.text.trim();
            if text.is_empty() {
                return None;
            }
            let start = word.start_ms.clamp(0, length);
            let end = word.end_ms.clamp(start, length);
            Some(Word {
                w: text.to_string(),
                s: (offset + start) as f64 / 1000.0,
                e: (offset + end) as f64 / 1000.0,
            })
        })
        .collect()
}

/// The turns of a diarized track of `track_ms`: speakers named `S1`, `S2`…
/// by the model's number, in time order. The model groups its segments by
/// speaker and lets different speakers overlap; the order here is the same
/// for the same audio, as the identity of a diarization depends on it.
pub fn turns(spoken: &[Spoken], track_ms: u64) -> Vec<Turn> {
    let end_of_track = track_ms as i64;
    let mut kept: Vec<(i64, i64, i32)> = spoken
        .iter()
        .filter(|segment| segment.speaker >= 1)
        .filter_map(|segment| {
            let start = segment.start_ms.max(0);
            let end = segment.end_ms.min(end_of_track);
            (end > start).then_some((start, end, segment.speaker))
        })
        .collect();
    kept.sort();
    kept.into_iter()
        .map(|(start, end, speaker)| Turn {
            spk: format!("S{speaker}"),
            s: start as f64 / 1000.0,
            e: end as f64 / 1000.0,
        })
        .collect()
}

fn error(stage: &str, code: ErrorCode) -> Message {
    Message::Error {
        stage: stage.to_string(),
        code,
        message: String::new(),
    }
}

/// How a failed model call ends the run, after telling `analysis`.
fn ended_by(failure: Failure, stage: &str, analysis: &mut Analysis) -> Ended {
    match failure {
        Failure::Stopped => Ended::Cancelled,
        Failure::Missing => {
            analysis.absorb(error(stage, ErrorCode::ModelMissing));
            Ended::Crashed
        }
        Failure::Failed(kind) => {
            log::warn!("Meeting analysis: {stage} failed ({kind})");
            analysis.absorb(error(stage, ErrorCode::Crash));
            Ended::Crashed
        }
    }
}

/// The words of every track, then who spoke when on the track the words pick
/// (`diarized_track`). What is found goes into `analysis` as the helper's
/// messages would; `progress` gets the share of a phase done on a track.
///
/// Without a speaker model (`speakers` is `None`) the words are kept and the
/// analysis says the models are missing: one speaker per track, and the
/// analysis runs again once the model is there.
pub fn run(
    job: &Run,
    recognizer: &mut dyn Recognizer,
    speakers: Option<&mut dyn Diarizer>,
    analysis: &mut Analysis,
    progress: &mut dyn FnMut(Phase, Track, f64),
) -> Ended {
    for (track, path) in job.tracks {
        let samples = match job.audio.read(path) {
            Ok(samples) => samples,
            Err(_) => {
                analysis.absorb(error("asr", ErrorCode::Io));
                return Ended::Crashed;
            }
        };
        let mut words: Vec<Word> = Vec::new();
        let pieces = job.audio.pieces(&samples);
        let total: usize = pieces.iter().map(|piece| piece.len()).sum();
        let mut done = 0;
        progress(Phase::Asr, *track, 0.0);
        for range in pieces {
            if (job.stop)() {
                return Ended::Cancelled;
            }
            let piece = &samples[range.clone()];
            match recognizer.hear(&padded(piece)) {
                Ok(heard) => words.extend(words_at(
                    heard,
                    samples_ms(range.start),
                    samples_ms(piece.len()),
                )),
                Err(failure) => return ended_by(failure, "asr", analysis),
            }
            done += piece.len();
            progress(Phase::Asr, *track, done as f64 / total.max(1) as f64);
        }
        words.sort_by(|a, b| a.s.total_cmp(&b.s));
        analysis.absorb(Message::Words {
            track: *track,
            words: crate::engine::local::unknown::words_without_unknown(words),
        });
    }

    let diarize = diarized_track(&analysis.words, job.by_levels);
    analysis.diarized = Some(diarize);
    let Some(speakers) = speakers else {
        analysis.absorb(error("diarize", ErrorCode::ModelMissing));
        return Ended::Done;
    };
    let Some((_, path)) = job.tracks.iter().find(|(track, _)| *track == diarize) else {
        return Ended::Done;
    };
    if (job.stop)() {
        return Ended::Cancelled;
    }
    let samples = match job.audio.read(path) {
        Ok(samples) => samples,
        Err(_) => {
            analysis.absorb(error("diarize", ErrorCode::Io));
            return Ended::Crashed;
        }
    };
    progress(Phase::Diarize, diarize, 0.0);
    let segments = if job.audio.is_silent(&samples) {
        // Nobody to tell apart: a model fed silence invents speakers.
        Vec::new()
    } else {
        match speakers.tell_apart_reporting(&samples, &mut |fraction| {
            progress(Phase::Diarize, diarize, fraction)
        }) {
            Ok(spoken) => turns(&spoken, samples_ms(samples.len())),
            Err(failure) => return ended_by(failure, "diarize", analysis),
        }
    };
    analysis.absorb(Message::Diarization {
        track: diarize,
        segments,
    });
    progress(Phase::Diarize, diarize, 1.0);
    Ended::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::store::SpeakerAnalysis;
    use std::cell::Cell;

    const SECOND: usize = SAMPLES_PER_SECOND;

    /// Tracks of silence of the length the path names (`mic-40.wav`), cut in
    /// pieces of 20 s.
    struct Fake {
        silent: bool,
        unreadable: Option<&'static str>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                silent: false,
                unreadable: None,
            }
        }
    }

    fn seconds_in(path: &Path) -> usize {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.rsplit('-').next())
            .and_then(|seconds| seconds.parse().ok())
            .unwrap()
    }

    impl TrackAudio for Fake {
        fn read(&self, path: &Path) -> anyhow::Result<Vec<f32>> {
            if self
                .unreadable
                .is_some_and(|name| path.to_string_lossy().contains(name))
            {
                anyhow::bail!("unreadable");
            }
            Ok(vec![0.1; seconds_in(path) * SECOND])
        }

        fn pieces(&self, samples: &[f32]) -> Vec<Range<usize>> {
            let piece = 20 * SECOND;
            (0..samples.len())
                .step_by(piece)
                .map(|start| start..(start + piece).min(samples.len()))
                .collect()
        }

        fn is_silent(&self, _: &[f32]) -> bool {
            self.silent
        }
    }

    /// Hears `per_piece` words in each piece, the first at 1 s into it, 1 s
    /// apart; keeps the length of every piece it was given.
    struct Words {
        per_piece: usize,
        lengths: Vec<usize>,
        fail_at: Option<(usize, Failure)>,
        extra: Vec<Heard>,
    }

    impl Words {
        fn new(per_piece: usize) -> Self {
            Self {
                per_piece,
                lengths: Vec::new(),
                fail_at: None,
                extra: Vec::new(),
            }
        }
    }

    impl Recognizer for Words {
        fn hear(&mut self, piece: &[f32]) -> Result<Vec<Heard>, Failure> {
            if let Some((at, failure)) = &self.fail_at {
                if self.lengths.len() == *at {
                    return Err(failure.clone());
                }
            }
            self.lengths.push(piece.len());
            let mut heard: Vec<Heard> = (0..self.per_piece as i64)
                .map(|index| Heard {
                    text: format!(" mot{index} "),
                    start_ms: 1_000 + index * 1_000,
                    end_ms: 1_500 + index * 1_000,
                })
                .collect();
            heard.extend(self.extra.clone());
            Ok(heard)
        }
    }

    /// Two speakers, the second first in the list as the model groups them by
    /// speaker, and a stray one that is not a speaker.
    struct Voices {
        seen: Vec<usize>,
        answer: Result<Vec<Spoken>, Failure>,
    }

    impl Voices {
        fn two() -> Self {
            Self {
                seen: Vec::new(),
                answer: Ok(vec![
                    Spoken {
                        speaker: 2,
                        start_ms: 5_000,
                        end_ms: 9_000,
                    },
                    Spoken {
                        speaker: 1,
                        start_ms: 0,
                        end_ms: 4_000,
                    },
                    Spoken {
                        speaker: 0,
                        start_ms: 0,
                        end_ms: 100,
                    },
                ]),
            }
        }
    }

    impl Diarizer for Voices {
        fn tell_apart(&mut self, track: &[f32]) -> Result<Vec<Spoken>, Failure> {
            self.seen.push(track.len());
            self.answer.clone()
        }
    }

    fn tracks(mic: usize, system: usize) -> Vec<(Track, PathBuf)> {
        vec![
            (Track::Mic, PathBuf::from(format!("mic-{mic}.wav"))),
            (Track::System, PathBuf::from(format!("system-{system}.wav"))),
        ]
    }

    fn never() -> bool {
        false
    }

    fn analyse(
        tracks: &[(Track, PathBuf)],
        by_levels: Track,
        audio: &Fake,
        recognizer: &mut Words,
        speakers: Option<&mut dyn Diarizer>,
    ) -> (Ended, Analysis, Vec<(Phase, Track, f64)>) {
        let mut analysis = Analysis::default();
        let mut steps = Vec::new();
        let ended = run(
            &Run {
                tracks,
                by_levels,
                audio,
                stop: &never,
            },
            recognizer,
            speakers,
            &mut analysis,
            &mut |phase, track, fraction| steps.push((phase, track, fraction)),
        );
        (ended, analysis, steps)
    }

    #[test]
    fn words_sit_at_their_time_on_the_track_not_in_their_piece() {
        let tracks = tracks(40, 0);
        let mut words = Words::new(2);
        let (ended, analysis, _) =
            analyse(&tracks[..1], Track::Mic, &Fake::new(), &mut words, None);
        assert_eq!(ended, Ended::Done);
        let heard: Vec<(String, f64, f64)> = analysis.words[&Track::Mic]
            .iter()
            .map(|word| (word.w.clone(), word.s, word.e))
            .collect();
        assert_eq!(
            heard,
            [
                ("mot0".to_string(), 1.0, 1.5),
                ("mot1".to_string(), 2.0, 2.5),
                // The second piece starts 20 s into the track.
                ("mot0".to_string(), 21.0, 21.5),
                ("mot1".to_string(), 22.0, 22.5),
            ]
        );
    }

    #[test]
    fn a_word_outside_its_piece_is_brought_inside_and_the_words_stay_in_order() {
        let tracks = tracks(10, 0);
        let mut words = Words::new(1);
        words.extra = vec![
            Heard {
                text: "late".into(),
                start_ms: 9_990_000,
                end_ms: 9_999_000,
            },
            Heard {
                text: "early".into(),
                start_ms: -500,
                end_ms: -100,
            },
            Heard {
                text: "  ".into(),
                start_ms: 2_000,
                end_ms: 2_100,
            },
            Heard {
                text: "backwards".into(),
                start_ms: 4_000,
                end_ms: 3_000,
            },
        ];
        let (_, analysis, _) = analyse(&tracks[..1], Track::Mic, &Fake::new(), &mut words, None);
        let heard = &analysis.words[&Track::Mic];
        let texts: Vec<&str> = heard.iter().map(|word| word.w.as_str()).collect();
        assert_eq!(texts, ["early", "mot0", "backwards", "late"]);
        for word in heard {
            assert!(word.s <= word.e, "{word:?}");
            assert!((0.0..=10.0).contains(&word.s) && word.e <= 10.0, "{word:?}");
        }
        assert!(heard.windows(2).all(|pair| pair[0].s <= pair[1].s));
    }

    #[test]
    fn a_short_piece_is_padded_to_half_a_second() {
        let tracks = tracks(0, 0);
        // 0.2 s of audio: the fake path says seconds, so cut it by hand.
        struct Short;
        impl TrackAudio for Short {
            fn read(&self, _: &Path) -> anyhow::Result<Vec<f32>> {
                Ok(vec![0.1; SECOND / 5])
            }
            fn pieces(&self, samples: &[f32]) -> Vec<Range<usize>> {
                std::iter::once(0..samples.len()).collect()
            }
            fn is_silent(&self, _: &[f32]) -> bool {
                false
            }
        }
        let mut words = Words::new(1);
        let mut analysis = Analysis::default();
        run(
            &Run {
                tracks: &tracks[..1],
                by_levels: Track::Mic,
                audio: &Short,
                stop: &never,
            },
            &mut words,
            None,
            &mut analysis,
            &mut |_, _, _| {},
        );
        assert_eq!(words.lengths, [SECOND / 2]);
        // The word is brought inside the 0.2 s that were real.
        assert!(analysis.words[&Track::Mic][0].e <= 0.2);
    }

    #[test]
    fn the_track_the_words_pick_is_told_apart_and_its_turns_are_clean() {
        // 20 words on system audio: the remote side spoke there.
        let tracks = tracks(20, 20);
        let mut words = Words::new(20);
        let mut voices = Voices::two();
        let (ended, analysis, steps) = analyse(
            &tracks,
            Track::Mic,
            &Fake::new(),
            &mut words,
            Some(&mut voices),
        );
        assert_eq!(ended, Ended::Done);
        assert_eq!(analysis.diarized, Some(Track::System));
        assert_eq!(voices.seen, [20 * SECOND]);
        let turns = analysis.turns.as_ref().unwrap();
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn.spk.as_str())
                .collect::<Vec<_>>(),
            ["S1", "S2"]
        );
        assert_eq!((turns[1].s, turns[1].e), (5.0, 9.0));
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Separated);
        assert_eq!(analysis.words.len(), 2);
        // Progress runs from 0 to 1 on each phase.
        assert!(steps.contains(&(Phase::Asr, Track::Mic, 1.0)));
        assert!(steps.contains(&(Phase::Diarize, Track::System, 1.0)));
    }

    #[test]
    fn a_room_with_next_to_nothing_on_system_audio_tells_the_microphone_apart() {
        let tracks = tracks(40, 20);
        let mut words = Words::new(1);
        let mut voices = Voices::two();
        let (_, analysis, _) = analyse(
            &tracks,
            Track::System,
            &Fake::new(),
            &mut words,
            Some(&mut voices),
        );
        // One word on system audio and two on the microphone.
        assert_eq!(analysis.diarized, Some(Track::Mic));
        assert_eq!(voices.seen, [40 * SECOND]);
    }

    #[test]
    fn without_the_speaker_model_the_words_are_kept_and_the_models_are_missing() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(3);
        let (ended, analysis, _) = analyse(&tracks, Track::Mic, &Fake::new(), &mut words, None);
        assert_eq!(ended, Ended::Done);
        assert_eq!(analysis.words.len(), 2);
        assert_eq!(analysis.turns, None);
        assert!(analysis.models_missing);
        assert_eq!(analysis.outcome(), SpeakerAnalysis::NoModels);
    }

    #[test]
    fn silence_is_not_given_to_the_speaker_model() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(20);
        let mut voices = Voices::two();
        let fake = Fake {
            silent: true,
            ..Fake::new()
        };
        let (ended, analysis, _) =
            analyse(&tracks, Track::Mic, &fake, &mut words, Some(&mut voices));
        assert_eq!(ended, Ended::Done);
        assert!(voices.seen.is_empty());
        assert_eq!(analysis.turns, Some(vec![]));
        // Nobody to tell apart: the user may try again.
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);
    }

    #[test]
    fn a_failing_speech_model_ends_the_run_and_keeps_the_words_before_it() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(2);
        // The mic track (one piece) is heard, the system track fails.
        words.fail_at = Some((1, Failure::Failed("Backend".into())));
        let (ended, analysis, _) = analyse(&tracks, Track::Mic, &Fake::new(), &mut words, None);
        assert_eq!(ended, Ended::Crashed);
        assert_eq!(analysis.words.len(), 1);
        assert!(analysis.words.contains_key(&Track::Mic));
        assert!(!analysis.models_missing);
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);
    }

    #[test]
    fn a_speech_model_that_went_missing_asks_for_the_models_again() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(2);
        words.fail_at = Some((0, Failure::Missing));
        let (ended, analysis, _) = analyse(&tracks, Track::Mic, &Fake::new(), &mut words, None);
        assert_eq!(ended, Ended::Crashed);
        assert!(analysis.models_missing);
        assert!(analysis.words.is_empty());
    }

    #[test]
    fn a_failing_speaker_model_keeps_the_words() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(20);
        let mut voices = Voices::two();
        voices.answer = Err(Failure::Failed("OutOfMemory".into()));
        let (ended, analysis, _) = analyse(
            &tracks,
            Track::Mic,
            &Fake::new(),
            &mut words,
            Some(&mut voices),
        );
        assert_eq!(ended, Ended::Crashed);
        assert_eq!(analysis.words.len(), 2);
        assert_eq!(analysis.turns, None);
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);
    }

    #[test]
    fn an_unreadable_track_fails_the_analysis_not_the_meeting() {
        let tracks = tracks(20, 20);
        let mut words = Words::new(2);
        let fake = Fake {
            unreadable: Some("system"),
            ..Fake::new()
        };
        let (ended, analysis, _) = analyse(&tracks, Track::Mic, &fake, &mut words, None);
        assert_eq!(ended, Ended::Crashed);
        assert!(analysis.words.contains_key(&Track::Mic));
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);
    }

    #[test]
    fn a_stopped_run_ends_as_cancelled_between_pieces() {
        let tracks = tracks(60, 0);
        let mut words = Words::new(1);
        let heard = Cell::new(0);
        let mut analysis = Analysis::default();
        let stop = || heard.get() >= 2;
        struct Counting<'a>(&'a mut Words, &'a Cell<usize>);
        impl Recognizer for Counting<'_> {
            fn hear(&mut self, piece: &[f32]) -> Result<Vec<Heard>, Failure> {
                self.1.set(self.1.get() + 1);
                self.0.hear(piece)
            }
        }
        let ended = run(
            &Run {
                tracks: &tracks[..1],
                by_levels: Track::Mic,
                audio: &Fake::new(),
                stop: &stop,
            },
            &mut Counting(&mut words, &heard),
            None,
            &mut analysis,
            &mut |_, _, _| {},
        );
        assert_eq!(ended, Ended::Cancelled);
        assert_eq!(words.lengths.len(), 2);
        assert!(analysis.words.is_empty());

        // A model that was stopped from outside ends the run the same way.
        let mut stopped = Words::new(1);
        stopped.fail_at = Some((0, Failure::Stopped));
        let (ended, _, _) = analyse(&tracks[..1], Track::Mic, &Fake::new(), &mut stopped, None);
        assert_eq!(ended, Ended::Cancelled);
    }

    #[test]
    fn turns_are_numbered_clamped_and_ordered_the_same_way_every_time() {
        let spoken = vec![
            Spoken {
                speaker: 3,
                start_ms: 2_000,
                end_ms: 2_000,
            },
            Spoken {
                speaker: 2,
                start_ms: 6_000,
                end_ms: 99_000,
            },
            Spoken {
                speaker: 1,
                start_ms: -300,
                end_ms: 4_000,
            },
            Spoken {
                speaker: 1,
                start_ms: 9_000,
                end_ms: 12_000,
            },
            Spoken {
                speaker: 4,
                start_ms: 20_000,
                end_ms: 21_000,
            },
            Spoken {
                speaker: -1,
                start_ms: 0,
                end_ms: 1_000,
            },
        ];
        let result = turns(&spoken, 10_000);
        let shown: Vec<(&str, f64, f64)> = result
            .iter()
            .map(|turn| (turn.spk.as_str(), turn.s, turn.e))
            .collect();
        assert_eq!(
            shown,
            [("S1", 0.0, 4.0), ("S2", 6.0, 10.0), ("S1", 9.0, 10.0)]
        );
        let mut reversed = spoken.clone();
        reversed.reverse();
        assert_eq!(turns(&reversed, 10_000), result);
    }

    /// The model numbers speakers as it meets them; there is no maximum, and
    /// two digits name a speaker as well as one.
    #[test]
    fn twelve_speakers_are_named_s1_to_s12_in_the_order_they_start() {
        let spoken: Vec<Spoken> = (1..=12)
            .rev()
            .map(|speaker| Spoken {
                speaker,
                start_ms: i64::from(speaker) * 5_000,
                end_ms: i64::from(speaker) * 5_000 + 4_000,
            })
            .collect();
        let result = turns(&spoken, 70_000);
        let names: Vec<String> = result.iter().map(|turn| turn.spk.clone()).collect();
        let expected: Vec<String> = (1..=12).map(|speaker| format!("S{speaker}")).collect();
        assert_eq!(names, expected);
        assert!(result.windows(2).all(|pair| pair[0].s < pair[1].s));
    }

    #[test]
    fn a_run_keeps_the_turns_of_more_than_four_speakers() {
        let mut speakers = Voices::two();
        speakers.answer = Ok((1..=8)
            .map(|speaker| Spoken {
                speaker,
                start_ms: i64::from(speaker - 1) * 4_000,
                end_ms: i64::from(speaker) * 4_000,
            })
            .collect());
        let (ended, analysis, _) = analyse(
            &tracks(40, 40),
            Track::System,
            &Fake::new(),
            &mut Words::new(1),
            Some(&mut speakers),
        );
        assert_eq!(ended, Ended::Done);
        let turns = analysis.turns.expect("no turns");
        let names: std::collections::BTreeSet<&str> =
            turns.iter().map(|turn| turn.spk.as_str()).collect();
        assert_eq!(names.len(), 8);
        assert!(names.contains("S8"));
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Separated);
    }

    #[test]
    fn a_speaker_model_that_reports_its_progress_has_it_passed_on() {
        struct Reporting;
        impl Diarizer for Reporting {
            fn tell_apart(&mut self, _: &[f32]) -> Result<Vec<Spoken>, Failure> {
                Ok(Vec::new())
            }
            fn tell_apart_reporting(
                &mut self,
                _: &[f32],
                progress: &mut dyn FnMut(f64),
            ) -> Result<Vec<Spoken>, Failure> {
                progress(0.25);
                progress(0.75);
                Ok(Vec::new())
            }
        }
        let (_, _, steps) = analyse(
            &tracks(40, 40),
            Track::System,
            &Fake::new(),
            &mut Words::new(1),
            Some(&mut Reporting),
        );
        let diarize: Vec<f64> = steps
            .iter()
            .filter(|(phase, _, _)| *phase == Phase::Diarize)
            .map(|(_, _, fraction)| *fraction)
            .collect();
        assert_eq!(diarize, [0.0, 0.25, 0.75, 1.0]);
    }

    #[test]
    fn the_time_limit_grows_with_the_audio_and_never_falls_below_the_base() {
        assert_eq!(time_limit(&[]), LIMIT_BASE);
        let hour = 3_600_000;
        // Two tracks of an hour, one read again: three hours, twice.
        assert_eq!(
            time_limit(&[hour, hour]),
            LIMIT_BASE + Duration::from_millis(3 * hour * 2)
        );
        assert!(time_limit(&[hour]) < time_limit(&[hour, hour]));
    }

    /// The speaker pass costs one window of the segmentation and one of the
    /// encoder per 2 s of audio: 0.057 s per second of audio on 2 threads of a
    /// server CPU (`docs/windows-meetings.md`). A processor 20 times slower
    /// than that (1.2 s per second), that also recognises both tracks at real
    /// time, still ends a 3-hour meeting within the limit; a hung model does
    /// not.
    #[test]
    fn a_three_hour_meeting_on_a_very_slow_processor_fits_the_limit() {
        let hour = 3_600_000u64;
        let three_hours = 3 * hour;
        let limit = time_limit(&[three_hours, three_hours]);
        let recognition = Duration::from_millis(2 * three_hours);
        let speakers = Duration::from_millis(three_hours * 12 / 10);
        assert!(recognition + speakers < limit);
        assert!(limit < Duration::from_millis(6 * three_hours + 60 * 60 * 1000));
    }

    #[test]
    fn the_pieces_the_recogniser_hears_are_the_ones_the_audio_cut() {
        let tracks = tracks(50, 0);
        let mut words = Words::new(0);
        analyse(&tracks[..1], Track::Mic, &Fake::new(), &mut words, None);
        assert_eq!(words.lengths, [20 * SECOND, 20 * SECOND, 10 * SECOND]);
    }
}
