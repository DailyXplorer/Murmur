use super::saver::Audio;
use crate::chinese_script::in_chosen_script;
use crate::meetings::align::timing::{sentence_times, time_chunk, words_in};
use crate::meetings::align::{TimedToken, TimedWord};
use crate::meetings::analysis::Analysis;
use crate::meetings::bleed::{remove_echoes, WordTimes};
use crate::meetings::capture::route::OutputRoute;
use crate::meetings::merge::{merge, Track, Unit, PLACEHOLDER};
use crate::meetings::protocol::{segments, timed_words};
use crate::meetings::speakers::{self, speaker_rows};
use crate::meetings::spread::{spread, voiced_intervals};
use crate::meetings::store::{ChunkRow, ChunkStatus, MeetingSpeaker, MeetingTurn};
use crate::meetings::tokens::{sentences, tokenize};
use std::collections::HashMap;
use std::time::Duration;

/// The key of a track's only speaker when its speakers are not told apart:
/// the microphone is the user, system audio the remote participants, or,
/// after `room_voices` told apart on the microphone, one voice more.
fn single_speaker(track: Track, room_voices: usize) -> String {
    match track {
        Track::Mic => "me".to_string(),
        Track::System => format!("s{}", room_voices + 1),
    }
}

/// One log line with how long each stage took, against the meeting's length.
/// Transcription and analysis run side by side, so stages add up to more
/// than `total`.
pub(super) fn timings_line(audio_ms: u64, stages: &[(&str, Duration)], total: Duration) -> String {
    let mut parts = vec![format!("audio {:.1} min", audio_ms as f64 / 60_000.0)];
    parts.extend(
        stages
            .iter()
            .map(|(stage, took)| format!("{stage} {:.1} s", took.as_secs_f64())),
    );
    parts.push(format!("total {:.1} s", total.as_secs_f64()));
    let mut line = parts.join(", ");
    if audio_ms > 0 && !total.is_zero() {
        let speed = audio_ms as f64 / 1000.0 / total.as_secs_f64();
        line.push_str(&format!(" ({speed:.0}x real time)"));
    }
    line
}

/// How much track audio a transcript leaves out, in milliseconds.
pub(super) fn untranscribed_ms(turns: &[MeetingTurn]) -> i64 {
    turns
        .iter()
        .filter(|turn| turn.speaker_key == PLACEHOLDER)
        .map(|turn| turn.end_ms - turn.start_ms)
        .sum()
}

/// The transcript built from the chunks Codex transcribed.
pub(super) struct Built {
    pub(super) turns: Vec<MeetingTurn>,
    pub(super) speakers: Vec<MeetingSpeaker>,
    /// The diarization the speaker keys come from; `None` when each track
    /// has one speaker.
    pub(super) identity: Option<String>,
    /// Some parts could not be transcribed.
    pub(super) missing: bool,
    /// Share of words placed at a measured time, over the chunks that had
    /// on-device words.
    pub(super) score: Option<f32>,
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

/// The transcript, with Chinese in the script `language` chose. Pieces keep
/// the script Codex wrote, so a transcript built again follows the choice.
/// `route` is where the meeting played sound.
pub(super) fn build_transcript(
    chunks: &[ChunkRow],
    audio: &HashMap<Track, Audio>,
    analysis: Option<&Analysis>,
    diarize: Track,
    language: Option<&str>,
    route: OutputRoute,
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
    let numbers = turns.as_deref().map(speakers::number);
    let room_voices = numbers
        .as_ref()
        .filter(|_| diarize == Track::Mic)
        .map_or(0, HashMap::len);
    let mut identity = None;
    let mut units = Vec::new();
    let mut missing = false;
    let mut score = (0.0, 0);
    // The system words as placed with Codex's text, for when none were
    // heard on device.
    let mut placed_system: Vec<TimedWord> = Vec::new();
    for track in [Track::System, Track::Mic] {
        let Some(track_audio) = audio.get(&track) else {
            continue;
        };
        let mut stream: Vec<TimedToken> = Vec::new();
        for chunk in chunks.iter().filter(|chunk| chunk.track == track) {
            match (chunk.status, chunk.text.as_deref()) {
                (ChunkStatus::Done, Some(text)) => stream.extend(place_chunk(
                    &in_chosen_script(language, text),
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
        let keys: Vec<String> = match (diarized, &numbers, &turns) {
            (Some(raw), Some(numbers), Some(turns)) => {
                identity = Some(speakers::identity(turns));
                raw.iter().map(|speaker| numbers[speaker].clone()).collect()
            }
            _ => vec![single_speaker(track, room_voices); stream.len()],
        };
        if track == Track::System {
            placed_system = stream
                .iter()
                .filter(|timed| !timed.token.key.is_empty())
                .map(|timed| TimedWord {
                    key: timed.token.key.clone(),
                    start_ms: timed.start_ms,
                    end_ms: timed.end_ms,
                })
                .collect();
        }
        units.extend(speakers::units(track, stream, &keys));
    }
    let (system_words, times) = match words.get(&Track::System) {
        Some(heard) => (heard.as_slice(), WordTimes::Heard),
        None => (placed_system.as_slice(), WordTimes::Placed),
    };
    if route == OutputRoute::Headphones {
        log::info!("The meeting played on headphones: no echo to remove");
    } else if let (Some(mic), Some(system)) = (audio.get(&Track::Mic), audio.get(&Track::System)) {
        let mic_chunk_starts: Vec<u64> = chunks
            .iter()
            .filter(|chunk| chunk.track == Track::Mic)
            .map(|chunk| chunk.start_ms)
            .collect();
        let removed = remove_echoes(
            &mut units,
            &mic_chunk_starts,
            system_words,
            times,
            &mic.levels.levels,
            &system.levels.levels,
        );
        if removed > 0 {
            log::info!(
                "Removed {removed} echo(es) of system audio from the microphone ({times:?} times)"
            );
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
    let speakers = speaker_rows(
        turns
            .iter()
            .map(|turn| turn.speaker_key.as_str())
            .filter(|key| *key != PLACEHOLDER),
        has_me,
    );
    Built {
        speakers,
        identity,
        missing,
        score: (score.1 > 0).then(|| score.0 / score.1 as f32),
        turns,
    }
}
