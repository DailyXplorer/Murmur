//! Interleaves the sentences of both tracks into the bubbles of the
//! transcript, in the order they were said. Sentences are ordered whole, so
//! a short "Yes." from a remote speaker falls between two of the user's
//! sentences instead of cutting one in half.

use super::tokens::{join, Token};

/// The speaker key of a notice for audio that could not be transcribed.
pub const PLACEHOLDER: &str = "_system";
/// A long sentence that another track speaks into is split at a comma.
const LONG_UNIT_MS: u64 = 20_000;
/// A pause this long starts a new bubble for the same speaker.
const BUBBLE_GAP_MS: u64 = 2_500;
/// A bubble this long takes no more sentences.
const BUBBLE_WORDS: usize = 90;

/// Ties go to system audio: a remote question comes before the answer
/// captured at the same moment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Track {
    System,
    Mic,
}

/// One sentence (or sentence piece) from one speaker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub track: Track,
    pub speaker: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub tokens: Vec<Token>,
    pub low_confidence: bool,
}

impl Unit {
    /// Marks `[start_ms, end_ms)` of a track as not transcribed.
    pub fn placeholder(track: Track, start_ms: u64, end_ms: u64) -> Self {
        Self {
            track,
            speaker: PLACEHOLDER.to_string(),
            start_ms,
            end_ms,
            tokens: Vec::new(),
            low_confidence: false,
        }
    }

    fn is_placeholder(&self) -> bool {
        self.speaker == PLACEHOLDER
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bubble {
    pub speaker: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub low_confidence: bool,
}

fn ends_clause(token: &Token) -> bool {
    token.text.ends_with([',', ';', ':', '，', '；', '、'])
}

/// Splits a long unit at the commas nearest the moments listed in `cuts`.
/// Token times are estimated from their length across the unit.
fn split_at_commas(unit: Unit, cuts: &[u64]) -> Vec<Unit> {
    let duration = unit.end_ms - unit.start_ms;
    let total: usize = unit.tokens.iter().map(Token::weight).sum();
    let mut weight = 0;
    // (index of the first token after the comma, estimated time there)
    let boundaries: Vec<(usize, u64)> = unit
        .tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            weight += token.weight();
            (ends_clause(token) && index + 1 < unit.tokens.len()).then(|| {
                let at = unit.start_ms + duration * weight as u64 / total.max(1) as u64;
                (index + 1, at)
            })
        })
        .collect();
    let mut splits: Vec<(usize, u64)> = cuts
        .iter()
        .filter_map(|cut| {
            boundaries
                .iter()
                .min_by_key(|(_, at)| at.abs_diff(*cut))
                .copied()
        })
        .collect();
    splits.sort_unstable();
    splits.dedup();
    if splits.is_empty() {
        return vec![unit];
    }
    let piece = |start_ms: u64, end_ms: u64, tokens: Vec<Token>| Unit {
        track: unit.track,
        speaker: unit.speaker.clone(),
        start_ms,
        end_ms,
        tokens,
        low_confidence: unit.low_confidence,
    };
    let mut pieces = Vec::new();
    let mut tokens = unit.tokens.clone();
    let mut taken = 0;
    let mut start = unit.start_ms;
    for (index, at) in splits {
        let rest = tokens.split_off(index - taken);
        pieces.push(piece(start, at, tokens));
        tokens = rest;
        taken = index;
        start = at;
    }
    pieces.push(piece(start, unit.end_ms, tokens));
    pieces
}

struct Building {
    bubble: Bubble,
    words: usize,
    low: usize,
}

impl Building {
    fn finish(self) -> Bubble {
        Bubble {
            low_confidence: self.low * 2 > self.words,
            ..self.bubble
        }
    }
}

/// Orders the units of both tracks by time and groups them into bubbles.
/// Every word of every unit comes out exactly once, in its track's order.
pub fn merge(units: Vec<Unit>) -> Vec<Bubble> {
    let starts: Vec<(Track, u64)> = units
        .iter()
        .filter(|unit| !unit.is_placeholder())
        .map(|unit| (unit.track, unit.start_ms))
        .collect();
    let mut pieces: Vec<Unit> = units
        .into_iter()
        .flat_map(|unit| {
            if unit.is_placeholder() || unit.end_ms - unit.start_ms <= LONG_UNIT_MS {
                return vec![unit];
            }
            let cuts: Vec<u64> = starts
                .iter()
                .filter(|(track, start)| {
                    *track != unit.track && *start > unit.start_ms && *start < unit.end_ms
                })
                .map(|(_, start)| *start)
                .collect();
            split_at_commas(unit, &cuts)
        })
        .collect();
    pieces.sort_by_key(|unit| (unit.start_ms, unit.track));

    let mut bubbles = Vec::new();
    let mut current: Option<Building> = None;
    for unit in pieces {
        if unit.tokens.is_empty() && !unit.is_placeholder() {
            continue;
        }
        let words = unit.tokens.len();
        let low = if unit.low_confidence { words } else { 0 };
        if let Some(building) = current.as_mut() {
            let joins = !unit.is_placeholder()
                && building.bubble.speaker == unit.speaker
                && unit.start_ms.saturating_sub(building.bubble.end_ms) < BUBBLE_GAP_MS
                && building.words < BUBBLE_WORDS;
            if joins {
                building.bubble.text.push(' ');
                building.bubble.text.push_str(&join(&unit.tokens));
                building.bubble.end_ms = building.bubble.end_ms.max(unit.end_ms);
                building.words += words;
                building.low += low;
                continue;
            }
        }
        if let Some(done) = current.take() {
            bubbles.push(done.finish());
        }
        current = Some(Building {
            bubble: Bubble {
                text: join(&unit.tokens),
                speaker: unit.speaker,
                start_ms: unit.start_ms,
                end_ms: unit.end_ms,
                low_confidence: false,
            },
            words,
            low,
        });
    }
    bubbles.extend(current.map(Building::finish));
    bubbles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::tokens::tokenize;

    fn unit(track: Track, start_ms: u64, end_ms: u64, text: &str) -> Unit {
        Unit {
            track,
            speaker: match track {
                Track::Mic => "me",
                Track::System => "s1",
            }
            .to_string(),
            start_ms,
            end_ms,
            tokens: tokenize(text),
            low_confidence: false,
        }
    }

    fn summary(bubbles: &[Bubble]) -> Vec<(&str, &str)> {
        bubbles
            .iter()
            .map(|b| (b.speaker.as_str(), b.text.as_str()))
            .collect()
    }

    #[test]
    fn a_short_reply_falls_between_two_sentences() {
        let bubbles = merge(vec![
            unit(Track::Mic, 0, 5_000, "Je pense qu'on peut livrer jeudi."),
            unit(Track::Mic, 5_000, 9_000, "Donc on y va."),
            unit(Track::System, 4_800, 5_200, "Oui."),
        ]);
        assert_eq!(
            summary(&bubbles),
            [
                ("me", "Je pense qu'on peut livrer jeudi."),
                ("s1", "Oui."),
                ("me", "Donc on y va."),
            ]
        );
    }

    #[test]
    fn at_the_same_moment_system_audio_comes_first() {
        let bubbles = merge(vec![
            unit(Track::Mic, 1_000, 2_000, "Bien sûr."),
            unit(Track::System, 1_000, 2_000, "Tu es là ?"),
        ]);
        assert_eq!(
            summary(&bubbles),
            [("s1", "Tu es là ?"), ("me", "Bien sûr.")]
        );
    }

    #[test]
    fn a_long_monologue_is_split_at_the_comma_nearest_an_interruption() {
        let bubbles = merge(vec![
            unit(
                Track::Mic,
                0,
                30_000,
                "Premier point le budget est validé, deuxième point le planning glisse, troisième point on recrute.",
            ),
            // The commas fall at about 10.9 s and 21.6 s.
            unit(Track::System, 20_000, 21_000, "D'accord."),
        ]);
        assert_eq!(
            summary(&bubbles),
            [
                (
                    "me",
                    "Premier point le budget est validé, deuxième point le planning glisse,"
                ),
                ("s1", "D'accord."),
                ("me", "troisième point on recrute."),
            ]
        );
        assert!(bubbles[0].end_ms <= bubbles[2].start_ms);
    }

    #[test]
    fn a_long_sentence_without_a_comma_stays_whole() {
        let bubbles = merge(vec![
            unit(
                Track::Mic,
                0,
                25_000,
                "Une très longue phrase sans aucune pause.",
            ),
            unit(Track::System, 10_000, 11_000, "Hm."),
        ]);
        assert_eq!(
            summary(&bubbles),
            [
                ("me", "Une très longue phrase sans aucune pause."),
                ("s1", "Hm.")
            ]
        );
    }

    #[test]
    fn bubbles_break_on_long_pauses_and_long_runs() {
        let bubbles = merge(vec![
            unit(Track::Mic, 0, 2_000, "Un."),
            unit(Track::Mic, 3_000, 4_000, "Deux."),
            unit(Track::Mic, 6_500, 7_000, "Trois."),
        ]);
        assert_eq!(summary(&bubbles), [("me", "Un. Deux."), ("me", "Trois.")]);
        assert_eq!((bubbles[0].start_ms, bubbles[0].end_ms), (0, 4_000));

        let long = vec!["mot"; 89].join(" ") + ".";
        let bubbles = merge(vec![
            unit(Track::Mic, 0, 20_000, &long),
            unit(Track::Mic, 20_000, 21_000, "Encore."),
            unit(Track::Mic, 21_000, 22_000, "Fin."),
        ]);
        assert_eq!(bubbles.len(), 2);
        assert_eq!(bubbles[1].text, "Fin.");
    }

    #[test]
    fn placeholders_keep_their_place_and_never_merge() {
        let bubbles = merge(vec![
            unit(Track::Mic, 0, 1_000, "Avant."),
            Unit::placeholder(Track::Mic, 600_000, 1_200_000),
            unit(Track::Mic, 1_200_000, 1_201_000, "Après."),
            unit(Track::System, 700_000, 701_000, "Pendant."),
        ]);
        assert_eq!(
            summary(&bubbles),
            [
                ("me", "Avant."),
                (PLACEHOLDER, ""),
                ("s1", "Pendant."),
                ("me", "Après."),
            ]
        );
    }

    #[test]
    fn low_confidence_follows_the_majority_of_words() {
        let mut first = unit(Track::Mic, 0, 1_000, "Un deux trois.");
        first.low_confidence = true;
        let bubbles = merge(vec![first, unit(Track::Mic, 1_000, 2_000, "Quatre.")]);
        assert!(bubbles[0].low_confidence);
    }

    #[test]
    fn every_word_comes_out_once_in_order_and_bubbles_never_go_back_in_time() {
        let mut state: u32 = 11;
        let mut next = move |bound: u32| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) % bound
        };
        for round in 0..60 {
            let mut units = Vec::new();
            for track in [Track::Mic, Track::System] {
                let mut at = 0;
                for index in 0..next(15) {
                    at += u64::from(next(8_000));
                    let length = u64::from(next(40_000)) + 1;
                    let words: Vec<String> = (0..next(30) + 1)
                        .map(|word| {
                            let comma = if next(5) == 0 { "," } else { "" };
                            format!("w{round}-{track:?}-{index}-{word}{comma}")
                        })
                        .collect();
                    units.push(unit(track, at, at + length, &(words.join(" ") + ".")));
                    at += length;
                }
            }
            let expected: Vec<Vec<String>> = ["me", "s1"]
                .iter()
                .map(|speaker| {
                    units
                        .iter()
                        .filter(|u| u.speaker == *speaker)
                        .flat_map(|u| u.tokens.iter().map(|t| t.text.clone()))
                        .collect()
                })
                .collect();
            let bubbles = merge(units);
            for (speaker, words) in ["me", "s1"].iter().zip(expected) {
                let got: Vec<String> = bubbles
                    .iter()
                    .filter(|b| b.speaker == *speaker)
                    .flat_map(|b| b.text.split(' ').map(str::to_string))
                    .filter(|word| !word.is_empty())
                    .collect();
                assert_eq!(got, words, "round {round}");
            }
            for pair in bubbles.windows(2) {
                assert!(pair[0].start_ms <= pair[1].start_ms, "round {round}");
            }
        }
    }
}
