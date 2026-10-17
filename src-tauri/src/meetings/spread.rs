//! Places the sentences of a Codex chunk on the timeline when no word
//! timings are available: they are laid end to end over the moments the
//! track holds speech, each taking time in proportion to its length. Good to
//! a few seconds, which is enough to interleave the tracks by sentence.

use super::chunking::FRAME_MS;
use super::tokens::Token;

/// Pauses shorter than this stay inside one stretch of speech.
const MAX_PAUSE_MS: u64 = 500;
/// Shorter bursts are clicks or breaths, not speech.
const MIN_SPEECH_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub start_ms: u64,
    pub end_ms: u64,
    pub tokens: Vec<Token>,
}

/// The stretches of `[start_ms, end_ms)` that hold speech, from the 20 ms
/// `levels` of the track. The whole range when none is found, so the text
/// of a chunk always has somewhere to go.
pub fn voiced_intervals(
    levels: &[f32],
    threshold: Option<f32>,
    start_ms: u64,
    end_ms: u64,
) -> Vec<(u64, u64)> {
    let mut intervals: Vec<(u64, u64)> = Vec::new();
    if let Some(threshold) = threshold {
        let first = (start_ms / FRAME_MS) as usize;
        let last = (end_ms.div_ceil(FRAME_MS) as usize).min(levels.len());
        for (frame, level) in levels.iter().enumerate().take(last).skip(first) {
            if *level < threshold {
                continue;
            }
            let from = (frame as u64 * FRAME_MS).max(start_ms);
            let to = ((frame as u64 + 1) * FRAME_MS).min(end_ms);
            match intervals.last_mut() {
                Some(last) if from <= last.1 + MAX_PAUSE_MS => last.1 = to,
                _ => intervals.push((from, to)),
            }
        }
    }
    intervals.retain(|(from, to)| to - from >= MIN_SPEECH_MS);
    if intervals.is_empty() && end_ms > start_ms {
        intervals.push((start_ms, end_ms));
    }
    intervals
}

/// The moment `offset` milliseconds of speech into `intervals`. A start
/// that falls on the end of one stretch moves to the start of the next.
fn at(intervals: &[(u64, u64)], offset: u64, is_end: bool) -> u64 {
    let mut remaining = offset;
    for (from, to) in intervals {
        let length = to - from;
        if remaining < length || (is_end && remaining == length) {
            return from + remaining;
        }
        remaining -= length;
    }
    intervals.last().map_or(0, |(_, to)| *to)
}

/// Lays `sentences` end to end over `intervals`, in order.
pub fn spread(sentences: Vec<Vec<Token>>, intervals: &[(u64, u64)]) -> Vec<Placed> {
    let voiced: u64 = intervals.iter().map(|(from, to)| to - from).sum();
    let weights: Vec<u64> = sentences
        .iter()
        .map(|sentence| sentence.iter().map(Token::weight).sum::<usize>() as u64)
        .collect();
    let total = weights.iter().sum::<u64>().max(1);
    let mut before = 0;
    sentences
        .into_iter()
        .zip(weights)
        .map(|(tokens, weight)| {
            let start = at(intervals, voiced * before / total, false);
            before += weight;
            let end = at(intervals, voiced * before / total, true).max(start);
            Placed {
                start_ms: start,
                end_ms: end,
                tokens,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::tokens::{join, sentences};

    const SPEECH: f32 = -20.0;
    const QUIET: f32 = -70.0;

    /// Levels for `total_ms` with speech in the given ranges.
    fn levels(total_ms: u64, speech: &[(u64, u64)]) -> Vec<f32> {
        (0..total_ms / FRAME_MS)
            .map(|frame| {
                let ms = frame * FRAME_MS;
                if speech.iter().any(|(from, to)| (*from..*to).contains(&ms)) {
                    SPEECH
                } else {
                    QUIET
                }
            })
            .collect()
    }

    #[test]
    fn speech_is_found_with_short_pauses_bridged_and_clicks_dropped() {
        let levels = levels(
            20_000,
            &[
                (1_000, 3_000),
                (3_300, 5_000),
                (8_000, 8_040),
                (12_000, 14_000),
            ],
        );
        assert_eq!(
            voiced_intervals(&levels, Some(-45.0), 0, 20_000),
            [(1_000, 5_000), (12_000, 14_000)]
        );
        assert_eq!(
            voiced_intervals(&levels, Some(-45.0), 2_010, 13_000),
            [(2_010, 5_000), (12_000, 13_000)]
        );
    }

    #[test]
    fn without_speech_the_whole_range_is_used() {
        let levels = levels(10_000, &[]);
        assert_eq!(
            voiced_intervals(&levels, Some(-45.0), 2_000, 6_000),
            [(2_000, 6_000)]
        );
        assert_eq!(voiced_intervals(&levels, None, 0, 10_000), [(0, 10_000)]);
        assert!(voiced_intervals(&levels, None, 5_000, 5_000).is_empty());
    }

    #[test]
    fn sentences_take_time_in_proportion_to_their_length() {
        let placed = spread(sentences("Un deux. Trois quatre."), &[(10_000, 20_000)]);
        assert_eq!(
            placed
                .iter()
                .map(|p| (p.start_ms, p.end_ms, join(&p.tokens)))
                .collect::<Vec<_>>(),
            [
                // Weights 8 and 13 (letters plus one per word).
                (10_000, 13_809, "Un deux.".to_string()),
                (13_809, 20_000, "Trois quatre.".to_string()),
            ]
        );
    }

    #[test]
    fn a_sentence_after_a_pause_starts_when_speech_resumes() {
        // Two sentences of equal weight over two stretches of speech.
        let placed = spread(sentences("Alpha. Gamma."), &[(0, 3_000), (9_000, 12_000)]);
        assert_eq!((placed[0].start_ms, placed[0].end_ms), (0, 3_000));
        assert_eq!((placed[1].start_ms, placed[1].end_ms), (9_000, 12_000));
    }

    #[test]
    fn every_word_is_placed_once_in_order() {
        let text = "Bon. Alors on fait le point ? Oui, très bien… Et la suite";
        let placed = spread(sentences(text), &[(500, 4_000), (6_000, 6_500)]);
        let words: Vec<String> = placed
            .iter()
            .flat_map(|p| p.tokens.iter().map(|t| t.text.clone()))
            .collect();
        assert_eq!(words.join(" "), text);
        for pair in placed.windows(2) {
            assert!(pair[0].start_ms <= pair[0].end_ms);
            assert!(pair[0].end_ms <= pair[1].start_ms);
        }
        assert_eq!(placed.last().unwrap().end_ms, 6_500);
    }
}
