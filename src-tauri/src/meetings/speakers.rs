//! Decides who said each word of a diarized track. A word goes to the
//! speaker whose turn overlaps it most, as WhisperX does; then each sentence
//! is smoothed, so a timing error of a few hundred milliseconds does not hand
//! a single word to someone else.

use std::collections::HashMap;

use super::align::TimedToken;
use super::merge::{Track, Unit};
use super::store::MeetingSpeaker;

/// A word with no turn under it takes a turn this close.
const NEAREST_MS: u64 = 1_000;
/// A sentence goes whole to a speaker who holds this share of it.
const SENTENCE_SHARE: f64 = 0.7;
/// Inside a mixed sentence, only runs this long stay separate.
const MIN_RUN_MS: u64 = 1_500;
const MIN_RUN_WORDS: usize = 3;
/// Doubtful times count for less when weighing a sentence.
const LOW_CONFIDENCE_WEIGHT: f64 = 0.3;
/// A "speaker" with less speech than this is a stray cluster.
const MIN_SPEAKER_MS: u64 = 2_000;
const MIN_SPEAKER_WORDS: usize = 5;

/// One turn of the diarization: who spoke over `[start_ms, end_ms)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub speaker: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

fn duration(token: &TimedToken) -> u64 {
    (token.end_ms - token.start_ms).max(1)
}

/// The speaker overlapping `token` most, or the nearest within a second.
fn speaker_at<'a>(token: &TimedToken, segments: &'a [Segment]) -> Option<&'a str> {
    // A word without length still sits at a moment.
    let (start, end) = (token.start_ms, token.end_ms.max(token.start_ms + 1));
    let mut overlap: HashMap<&str, u64> = HashMap::new();
    for segment in segments {
        let shared = end
            .min(segment.end_ms)
            .saturating_sub(start.max(segment.start_ms));
        if shared > 0 {
            *overlap.entry(segment.speaker.as_str()).or_default() += shared;
        }
    }
    if let Some((speaker, _)) = overlap
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
    {
        return Some(speaker);
    }
    segments
        .iter()
        .map(|segment| {
            let gap = segment
                .start_ms
                .saturating_sub(end)
                .max(start.saturating_sub(segment.end_ms));
            (gap, segment.speaker.as_str())
        })
        .filter(|(gap, _)| *gap <= NEAREST_MS)
        .min()
        .map(|(_, speaker)| speaker)
}

/// Gives each word of `stray` speakers the speaker of the nearest word
/// before it (or after, at the start) that is not stray.
fn absorb(speakers: &mut [String], stray: impl Fn(&str) -> bool) {
    let kept: Vec<Option<String>> = speakers
        .iter()
        .map(|speaker| (!stray(speaker.as_str())).then(|| speaker.clone()))
        .collect();
    let first = kept.iter().flatten().next().cloned();
    let mut last = first;
    for (speaker, keep) in speakers.iter_mut().zip(kept) {
        match keep {
            Some(keep) => last = Some(keep),
            None => {
                if let Some(last) = &last {
                    speaker.clone_from(last);
                }
            }
        }
    }
}

/// Smooths the speakers of one sentence in place.
fn smooth_sentence(tokens: &[TimedToken], speakers: &mut [String]) {
    let weight = |token: &TimedToken| {
        duration(token) as f64
            * if token.low_confidence {
                LOW_CONFIDENCE_WEIGHT
            } else {
                1.0
            }
    };
    let mut by_speaker: HashMap<&str, f64> = HashMap::new();
    for (token, speaker) in tokens.iter().zip(speakers.iter()) {
        *by_speaker.entry(speaker.as_str()).or_default() += weight(token);
    }
    let total: f64 = by_speaker.values().sum();
    let Some((top, top_weight)) = by_speaker
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(speaker, weight)| (speaker.to_string(), *weight))
    else {
        return;
    };
    if top_weight >= total * SENTENCE_SHARE {
        speakers
            .iter_mut()
            .for_each(|speaker| speaker.clone_from(&top));
        return;
    }
    // Runs of one speaker: (first token, past the last token).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, speaker) in speakers.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if speakers[run.0] == *speaker => run.1 = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    let long = |(from, to): (usize, usize)| {
        to - from >= MIN_RUN_WORDS && tokens[to - 1].end_ms - tokens[from].start_ms >= MIN_RUN_MS
    };
    if !runs.iter().any(|run| long(*run)) {
        speakers
            .iter_mut()
            .for_each(|speaker| speaker.clone_from(&top));
        return;
    }
    let short: Vec<bool> = runs.iter().map(|run| !long(*run)).collect();
    let mut owner: Vec<bool> = vec![false; speakers.len()];
    for ((from, to), is_short) in runs.iter().zip(&short) {
        owner[*from..*to].iter_mut().for_each(|o| *o = !is_short);
    }
    let mut marked: Vec<String> = speakers
        .iter()
        .zip(&owner)
        .map(|(speaker, owns)| {
            if *owns {
                speaker.clone()
            } else {
                String::new()
            }
        })
        .collect();
    absorb(&mut marked, str::is_empty);
    speakers.clone_from_slice(&marked);
}

/// The diarized speaker of each word of a track, in order. `None` when the
/// diarization has no turn at all, so the caller keeps one speaker.
pub fn assign(tokens: &[TimedToken], segments: &[Segment]) -> Option<Vec<String>> {
    let found: Vec<Option<&str>> = tokens
        .iter()
        .map(|token| speaker_at(token, segments))
        .collect();
    // Words with no turn near them follow the word before (or after).
    let mut speakers: Vec<String> = found
        .iter()
        .map(|speaker| speaker.unwrap_or_default().to_string())
        .collect();
    if speakers.iter().all(String::is_empty) {
        return None;
    }
    absorb(&mut speakers, str::is_empty);

    // Clusters with almost no speech are noise from the diarizer.
    let mut speech: HashMap<&str, (u64, usize)> = HashMap::new();
    for (token, speaker) in tokens.iter().zip(&speakers) {
        let entry = speech.entry(speaker.as_str()).or_default();
        entry.0 += duration(token);
        entry.1 += 1;
    }
    let stray: Vec<String> = speech
        .iter()
        .filter(|(_, (ms, words))| *ms < MIN_SPEAKER_MS && *words < MIN_SPEAKER_WORDS)
        .map(|(speaker, _)| speaker.to_string())
        .collect();
    if stray.len() < speech.len() {
        absorb(&mut speakers, |speaker| stray.iter().any(|s| s == speaker));
    }

    let mut from = 0;
    for (i, token) in tokens.iter().enumerate() {
        if token.token.sentence_end || i + 1 == tokens.len() {
            smooth_sentence(&tokens[from..=i], &mut speakers[from..=i]);
            from = i + 1;
        }
    }
    Some(speakers)
}

/// Stable keys `s1`, `s2`… for diarized speakers, in order of first word.
pub fn number(speakers: &[String]) -> HashMap<String, String> {
    let mut keys = HashMap::new();
    for speaker in speakers {
        let next = format!("s{}", keys.len() + 1);
        keys.entry(speaker.clone()).or_insert(next);
    }
    keys
}

/// The display row of speaker `key`. Remote speakers are numbered after the
/// user when the microphone is "Me".
pub fn speaker_row(key: &str, has_me: bool) -> MeetingSpeaker {
    let (name, color_index) = match key.strip_prefix('s').and_then(|n| n.parse::<u32>().ok()) {
        Some(n) => (format!("Speaker {}", n + u32::from(has_me)), n),
        None => ("Me".to_string(), 0),
    };
    MeetingSpeaker {
        key: key.to_string(),
        name: name.clone(),
        default_name: name,
        color_index,
    }
}

/// Splits a track's words into units at each sentence end and each change
/// of speaker.
pub fn units(track: Track, tokens: Vec<TimedToken>, speakers: &[String]) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    let mut low = 0;
    let mut closed = true;
    for (token, speaker) in tokens.into_iter().zip(speakers) {
        let continues = !closed && units.last().is_some_and(|unit| unit.speaker == *speaker);
        if !continues {
            if let Some(unit) = units.last_mut() {
                unit.low_confidence = low * 2 > unit.tokens.len();
            }
            low = 0;
            units.push(Unit {
                track,
                speaker: speaker.clone(),
                start_ms: token.start_ms,
                end_ms: token.end_ms,
                tokens: Vec::new(),
                low_confidence: false,
            });
        }
        let unit = units.last_mut().expect("a unit was just pushed");
        unit.end_ms = unit.end_ms.max(token.end_ms);
        low += usize::from(token.low_confidence);
        closed = token.token.sentence_end;
        unit.tokens.push(token.token);
    }
    if let Some(unit) = units.last_mut() {
        unit.low_confidence = low * 2 > unit.tokens.len();
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::tokens::tokenize;

    /// Words of `text`, each `step_ms` long, from `start_ms`.
    fn timed(text: &str, start_ms: u64, step_ms: u64) -> Vec<TimedToken> {
        tokenize(text)
            .into_iter()
            .enumerate()
            .map(|(i, token)| TimedToken {
                token,
                start_ms: start_ms + i as u64 * step_ms,
                end_ms: start_ms + (i as u64 + 1) * step_ms,
                low_confidence: false,
            })
            .collect()
    }

    fn segment(speaker: &str, start_ms: u64, end_ms: u64) -> Segment {
        Segment {
            speaker: speaker.to_string(),
            start_ms,
            end_ms,
        }
    }

    fn assigned(tokens: &[TimedToken], segments: &[Segment]) -> Vec<String> {
        assign(tokens, segments).unwrap()
    }

    #[test]
    fn each_word_goes_to_the_turn_overlapping_it_most() {
        let tokens = timed("Bonjour à tous. Salut merci beaucoup.", 0, 500);
        let segments = [segment("A", 0, 1_600), segment("B", 1_400, 3_000)];
        assert_eq!(assigned(&tokens, &segments), ["A", "A", "A", "B", "B", "B"]);
    }

    #[test]
    fn a_word_between_turns_takes_the_nearest_within_a_second() {
        let tokens = timed("Un deux trois.", 1_000, 1_000);
        // "deux" (2–3 s) touches no turn; B is 0.5 s away, A 1 s.
        let near = [segment("A", 0, 1_000), segment("B", 3_500, 5_000)];
        assert_eq!(speaker_at(&tokens[1], &near), Some("B"));
        let far = [segment("A", 5_000, 6_000)];
        assert_eq!(speaker_at(&tokens[1], &far), None);
    }

    #[test]
    fn words_far_from_any_turn_follow_their_neighbours() {
        let tokens = timed("Un deux. Trois quatre.", 0, 1_000);
        let segments = [segment("A", 1_000, 2_000)];
        // Only "deux" overlaps a turn; the others inherit A.
        assert_eq!(assigned(&tokens, &segments), ["A", "A", "A", "A"]);
        assert!(assign(&tokens, &[]).is_none());
    }

    #[test]
    fn a_sentence_mostly_from_one_speaker_goes_whole_to_them() {
        // Ten words; the turn change lands one word early.
        let tokens = timed("un deux trois quatre cinq six sept huit neuf dix.", 0, 400);
        let segments = [segment("A", 0, 3_600), segment("B", 3_600, 10_000)];
        // B holds the last word only: 10 %.
        assert_eq!(assigned(&tokens, &segments), vec!["A"; 10]);
    }

    #[test]
    fn a_sentence_with_two_long_runs_is_split_and_short_runs_are_absorbed() {
        // 2 s of A, a 0.4 s flicker to B, 2 s of A, then 2.4 s of B.
        let tokens = timed(
            "a1 a2 a3 a4 a5 b1 a6 a7 a8 a9 a10 c1 c2 c3 c4 c5 c6.",
            0,
            400,
        );
        let segments = [
            segment("A", 0, 2_000),
            segment("B", 2_000, 2_400),
            segment("A", 2_400, 4_400),
            segment("B", 4_400, 6_800),
        ];
        let speakers = assigned(&tokens, &segments);
        assert_eq!(speakers[..11], vec!["A"; 11]);
        assert_eq!(speakers[11..], vec!["B"; 6]);
    }

    #[test]
    fn doubtful_times_weigh_less_in_a_sentence() {
        let mut tokens = timed("un deux trois quatre.", 0, 1_000);
        // B holds half the time, but its words are doubtful.
        tokens[2].low_confidence = true;
        tokens[3].low_confidence = true;
        let segments = [segment("A", 0, 2_000), segment("B", 2_000, 4_000)];
        assert_eq!(assigned(&tokens, &segments), vec!["A"; 4]);
    }

    #[test]
    fn a_tiny_cluster_is_merged_into_its_neighbour() {
        let mut tokens = timed("Bonjour tout le monde.", 0, 500);
        tokens.extend(timed("Oui.", 2_000, 300));
        tokens.extend(timed("On commence par le budget.", 2_300, 500));
        let segments = [
            segment("A", 0, 2_000),
            // One short word: a stray cluster, not a person.
            segment("C", 2_000, 2_300),
            segment("A", 2_300, 5_000),
        ];
        assert!(assigned(&tokens, &segments).iter().all(|s| s == "A"));
    }

    #[test]
    fn speakers_are_numbered_by_first_word_after_me() {
        let keys = number(&["X".into(), "Y".into(), "X".into(), "Z".into()]);
        assert_eq!(keys["X"], "s1");
        assert_eq!(keys["Y"], "s2");
        assert_eq!(keys["Z"], "s3");
        assert_eq!(speaker_row("s2", true).name, "Speaker 3");
        assert_eq!(speaker_row("s2", false).name, "Speaker 2");
        assert_eq!(speaker_row("s2", false).color_index, 2);
        assert_eq!(speaker_row("me", true).name, "Me");
    }

    #[test]
    fn units_break_at_sentence_ends_and_speaker_changes() {
        let tokens = timed("Un deux. Trois quatre cinq.", 0, 1_000);
        let speakers: Vec<String> = ["s1", "s1", "s1", "s2", "s2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let units = units(Track::System, tokens, &speakers);
        let summary: Vec<(&str, u64, u64, usize)> = units
            .iter()
            .map(|u| (u.speaker.as_str(), u.start_ms, u.end_ms, u.tokens.len()))
            .collect();
        assert_eq!(
            summary,
            [
                ("s1", 0, 2_000, 2),
                ("s1", 2_000, 3_000, 1),
                ("s2", 3_000, 5_000, 2)
            ]
        );
    }
}
