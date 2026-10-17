//! Gives each Codex word of a chunk a time on the track, from the words the
//! on-device recogniser heard there. Words both heard alike are anchors and
//! take the measured time; the others are spread between their anchors by
//! length. Every Codex word comes out exactly once, in order.

use super::dp::{align, similarity, Step};
use super::{TimedToken, TimedWord};
use crate::meetings::tokens::Token;

/// Below this share of anchored words, the chunk's times are not trusted and
/// the caller spreads its sentences over the speech instead.
pub const MIN_SCORE: f32 = 0.35;
/// Words around which fewer than this share is anchored are flagged.
const LOW_LOCAL_SCORE: f32 = 0.2;
const LOCAL_WINDOW: usize = 25;
/// Average speaking time per character, for words before the first anchor
/// or after the last.
const MS_PER_WEIGHT: u64 = 60;
/// On-device words this close outside the chunk may still belong to it.
const EDGE_MS: u64 = 1_000;

/// The Codex words of a chunk placed on the track.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkTiming {
    pub tokens: Vec<TimedToken>,
    /// Anchored share of the words that can be compared, 0 to 1.
    pub score: f32,
    /// Words that can be compared: the weight of `score` in a meeting mean.
    pub comparable: usize,
}

impl ChunkTiming {
    pub fn trusted(&self) -> bool {
        self.score >= MIN_SCORE
    }
}

/// The words of `words` (sorted by start) that may belong to the chunk
/// `[start_ms, end_ms)`.
pub fn words_in(words: &[TimedWord], start_ms: u64, end_ms: u64) -> &[TimedWord] {
    let middle = |word: &TimedWord| (word.start_ms + word.end_ms) / 2;
    let first = words.partition_point(|word| middle(word) < start_ms.saturating_sub(EDGE_MS));
    let last = words.partition_point(|word| middle(word) < end_ms + EDGE_MS);
    &words[first..last.max(first)]
}

/// Which alignment steps are anchors: pairs that are surely the same word.
fn anchors(steps: &[Step], codex: &[&str], local: &[&str]) -> Vec<bool> {
    let sim: Vec<Option<f32>> = steps
        .iter()
        .map(|step| match step {
            Step::Pair(c, l) => Some(similarity(codex[*c], local[*l])),
            _ => None,
        })
        .collect();
    let exact = |index: Option<usize>| index.and_then(|i| sim[i]).is_some_and(|s| s >= 1.0);
    let neighbours = |i: usize| (i.checked_sub(1), (i + 1 < steps.len()).then_some(i + 1));
    // Short words like "de" match by chance unless a neighbour matches too.
    let strong: Vec<bool> = (0..steps.len())
        .map(|i| {
            let Step::Pair(c, _) = steps[i] else {
                return false;
            };
            let (before, after) = neighbours(i);
            sim[i].is_some_and(|s| s >= 0.8)
                && (codex[c].chars().count() >= 4 || exact(before) || exact(after))
        })
        .collect();
    // A looser match inside a run of anchors is the same word misspelt.
    (0..steps.len())
        .map(|i| {
            let (before, after) = neighbours(i);
            let anchored = |index: Option<usize>| index.is_some_and(|i| strong[i]);
            strong[i] || (sim[i].is_some_and(|s| s >= 0.6) && anchored(before) && anchored(after))
        })
        .collect()
}

/// Splits `[from, to]` among `weights`, in proportion.
fn share(from: u64, to: u64, weights: &[u64]) -> Vec<(u64, u64)> {
    let total: u64 = weights.iter().sum::<u64>().max(1);
    let span = to.saturating_sub(from);
    let mut before = 0;
    weights
        .iter()
        .map(|weight| {
            let start = from + span * before / total;
            before += weight;
            (start, from + span * before / total)
        })
        .collect()
}

/// Places the Codex `tokens` of the chunk `[start_ms, end_ms)` using the
/// on-device `words` heard there. Without any anchor, the score is 0 and the
/// times are the whole chunk.
pub fn time_chunk(
    tokens: Vec<Token>,
    words: &[TimedWord],
    start_ms: u64,
    end_ms: u64,
) -> ChunkTiming {
    // Punctuation alone cannot be compared; it is placed with its neighbours.
    let comparable: Vec<usize> = (0..tokens.len())
        .filter(|i| !tokens[*i].key.is_empty())
        .collect();
    let codex: Vec<&str> = comparable.iter().map(|i| tokens[*i].key.as_str()).collect();
    let local: Vec<&str> = words.iter().map(|word| word.key.as_str()).collect();
    // The on-device time of each anchored token.
    let mut measured: Vec<Option<(u64, u64)>> = vec![None; tokens.len()];
    if let Some(steps) = align(&codex, &local) {
        let mut last_start = 0;
        for (step, anchored) in steps.iter().zip(anchors(&steps, &codex, &local)) {
            if let (Step::Pair(c, l), true) = (step, anchored) {
                let word = &words[*l];
                // Times only move forward.
                if word.start_ms >= last_start {
                    last_start = word.start_ms;
                    measured[comparable[*c]] =
                        Some((word.start_ms, word.end_ms.max(word.start_ms)));
                }
            }
        }
    }
    let anchored_count = measured.iter().filter(|time| time.is_some()).count();
    let score = anchored_count as f32 / comparable.len().max(1) as f32;

    let weights: Vec<u64> = tokens.iter().map(|token| token.weight() as u64).collect();
    let mut times: Vec<(u64, u64)> = vec![(start_ms, end_ms); tokens.len()];
    let anchor_indices: Vec<usize> = (0..tokens.len())
        .filter(|i| measured[*i].is_some())
        .collect();
    if let (Some(&first), Some(&last)) = (anchor_indices.first(), anchor_indices.last()) {
        for i in &anchor_indices {
            times[*i] = measured[*i].unwrap_or((start_ms, end_ms));
        }
        // Before the first anchor: back from it at a speaking pace.
        let lead: u64 = weights[..first].iter().sum();
        let first_start = times[first].0;
        let from = first_start
            .saturating_sub(lead * MS_PER_WEIGHT)
            .max(start_ms)
            .min(first_start);
        for (i, time) in share(from, first_start, &weights[..first])
            .into_iter()
            .enumerate()
        {
            times[i] = time;
        }
        // Between anchors: the gap between them.
        for pair in anchor_indices.windows(2) {
            let (left, right) = (pair[0], pair[1]);
            let from = times[left].1;
            let to = times[right].0.max(from);
            for (offset, time) in share(from, to, &weights[left + 1..right])
                .into_iter()
                .enumerate()
            {
                times[left + 1 + offset] = time;
            }
        }
        // After the last anchor: on from it, at most to the chunk's end.
        let tail: u64 = weights[last + 1..].iter().sum();
        let last_end = times[last].1;
        let to = (last_end + tail * MS_PER_WEIGHT).min(end_ms).max(last_end);
        for (offset, time) in share(last_end, to, &weights[last + 1..])
            .into_iter()
            .enumerate()
        {
            times[last + 1 + offset] = time;
        }
    }

    // Words in a stretch with few anchors have a doubtful time.
    let anchored_flags: Vec<bool> = comparable.iter().map(|i| measured[*i].is_some()).collect();
    let mut low = vec![true; tokens.len()];
    for (position, token_index) in comparable.iter().enumerate() {
        let from = position.saturating_sub(LOCAL_WINDOW);
        let to = (position + LOCAL_WINDOW + 1).min(comparable.len());
        let window = &anchored_flags[from..to];
        let local_score =
            window.iter().filter(|anchored| **anchored).count() as f32 / window.len() as f32;
        low[*token_index] = local_score < LOW_LOCAL_SCORE;
    }
    // Punctuation follows the word before it.
    for i in 0..tokens.len() {
        if tokens[i].key.is_empty() {
            low[i] = i.checked_sub(1).is_none_or(|before| low[before]);
        }
    }

    ChunkTiming {
        tokens: tokens
            .into_iter()
            .zip(times)
            .zip(low)
            .map(|((token, (start_ms, end_ms)), low_confidence)| TimedToken {
                token,
                start_ms,
                end_ms,
                low_confidence,
            })
            .collect(),
        score,
        comparable: comparable.len(),
    }
}

/// Every token of each sentence takes the sentence's time: the fallback when
/// no word times can be trusted.
pub fn sentence_times(placed: Vec<crate::meetings::spread::Placed>) -> Vec<TimedToken> {
    placed
        .into_iter()
        .flat_map(|sentence| {
            let (start_ms, end_ms) = (sentence.start_ms, sentence.end_ms);
            sentence.tokens.into_iter().map(move |token| TimedToken {
                token,
                start_ms,
                end_ms,
                low_confidence: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::tokens::{key, tokenize};

    /// On-device words, one every `step_ms` from `start_ms`, each lasting
    /// most of its slot.
    fn heard(text: &str, start_ms: u64, step_ms: u64) -> Vec<TimedWord> {
        text.split(' ')
            .enumerate()
            .map(|(i, word)| TimedWord {
                key: key(word),
                start_ms: start_ms + i as u64 * step_ms,
                end_ms: start_ms + i as u64 * step_ms + step_ms * 3 / 4,
            })
            .collect()
    }

    fn times(timing: &ChunkTiming) -> Vec<(u64, u64)> {
        timing
            .tokens
            .iter()
            .map(|t| (t.start_ms, t.end_ms))
            .collect()
    }

    #[test]
    fn matching_words_take_the_measured_times() {
        let words = heard("bonjour tout le monde", 1_000, 400);
        let timing = time_chunk(tokenize("Bonjour tout le monde."), &words, 0, 10_000);
        assert_eq!(timing.score, 1.0);
        assert!(timing.trusted());
        assert_eq!(
            times(&timing),
            [
                (1_000, 1_300),
                (1_400, 1_700),
                (1_800, 2_100),
                (2_200, 2_500)
            ]
        );
        assert!(timing.tokens.iter().all(|t| !t.low_confidence));
    }

    #[test]
    fn a_number_written_in_digits_falls_between_its_neighbours() {
        // Codex writes "2030", the recogniser hears three words.
        let words = heard("objectif deux mille trente pour tous", 0, 500);
        let timing = time_chunk(tokenize("Objectif 2030 pour tous."), &words, 0, 10_000);
        let placed = times(&timing);
        assert_eq!(placed[0], (0, 375));
        // Between the end of "objectif" and the start of "pour".
        assert_eq!(placed[1], (375, 2_000));
        assert_eq!(placed[2], (2_000, 2_375));
        assert_eq!(timing.score, 0.75);
    }

    #[test]
    fn short_lone_matches_are_not_anchors() {
        // "de" pairs with "de" but nothing around it matches.
        let words = heard("xx de yy", 0, 500);
        let timing = time_chunk(tokenize("Alors de toute façon"), &words, 0, 4_000);
        assert_eq!(timing.score, 0.0);
        assert!(!timing.trusted());
        assert!(timing
            .tokens
            .iter()
            .all(|t| (t.start_ms, t.end_ms) == (0, 4_000)));
    }

    #[test]
    fn a_misspelt_word_inside_matches_is_anchored() {
        // "seminer" is only 0.67 like "séminaire", but sits between anchors.
        let words = heard("donc le seminer commence demain", 0, 500);
        let timing = time_chunk(
            tokenize("Donc le séminaire commence demain."),
            &words,
            0,
            4_000,
        );
        assert_eq!(timing.score, 1.0);
        assert_eq!(times(&timing)[2], (1_000, 1_375));
    }

    #[test]
    fn words_before_the_first_and_after_the_last_anchor_stay_in_the_chunk() {
        // The recogniser missed the first and last words.
        let words = heard("on avance sur le projet", 5_000, 500);
        let timing = time_chunk(
            tokenize("Bon, on avance sur le projet aujourd'hui."),
            &words,
            4_900,
            8_000,
        );
        let placed = times(&timing);
        // "Bon," has weight 4: 240 ms before "on", clamped to the chunk start.
        assert_eq!(placed[0], (4_900, 5_000));
        // "aujourd'hui." has weight 11: 660 ms after "projet" ends, cut at
        // the chunk's end.
        assert_eq!(placed[6], (7_375, 8_000));
        assert!(placed.windows(2).all(|pair| pair[0].1 <= pair[1].0));
    }

    #[test]
    fn punctuation_alone_is_placed_with_its_neighbours() {
        let words = heard("alors bon voila", 0, 1_000);
        let timing = time_chunk(tokenize("Alors – bon voilà"), &words, 0, 3_000);
        let placed = times(&timing);
        assert_eq!(placed[1], (750, 1_000));
        assert_eq!(timing.score, 1.0);
        assert!(!timing.tokens[1].low_confidence);
    }

    #[test]
    fn words_near_the_chunk_are_kept_and_others_dropped() {
        let words = heard("a b c d e f", 0, 1_000);
        let kept: Vec<&str> = words_in(&words, 2_000, 4_000)
            .iter()
            .map(|w| w.key.as_str())
            .collect();
        // Midpoints at 375, 1375, 2375…: within one second of [2 s, 4 s).
        assert_eq!(kept, ["b", "c", "d", "e"]);
        assert!(words_in(&words, 20_000, 30_000).is_empty());
    }

    #[test]
    fn no_word_is_ever_dropped_or_reordered() {
        let mut state: u32 = 3;
        let mut next = move |bound: u32| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) % bound
        };
        let vocabulary = [
            "projet", "budget", "jeudi", "de", "la", "équipe", "client", "?", "–",
        ];
        for _ in 0..200 {
            let codex: Vec<&str> = (0..next(40))
                .map(|_| vocabulary[next(9) as usize])
                .collect();
            let text = codex.join(" ");
            // The recogniser hears the same words with some changed or lost.
            let mut local: Vec<&str> = Vec::new();
            for word in &codex {
                if next(4) != 0 {
                    local.push(if next(5) == 0 { "euh" } else { word });
                }
            }
            let words = heard(&local.join(" "), 1_000, 300);
            let timing = time_chunk(tokenize(&text), &words, 0, 60_000);
            let out: Vec<&str> = timing
                .tokens
                .iter()
                .map(|t| t.token.text.as_str())
                .collect();
            assert_eq!(out.join(" "), text);
            for token in &timing.tokens {
                assert!(token.start_ms <= token.end_ms);
                assert!(token.end_ms <= 60_000);
            }
            for pair in timing.tokens.windows(2) {
                assert!(pair[0].start_ms <= pair[1].start_ms);
            }
        }
    }
}
