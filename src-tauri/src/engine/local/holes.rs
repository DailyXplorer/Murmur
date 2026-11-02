//! Speech the model skipped inside a piece. Over a whole stretch of a piece,
//! at its start or its end, Parakeet sometimes emits no word, mostly where a
//! sentence sits next to one from another speaker. The same stretch decoded
//! alone is heard, so it is transcribed again on its own and its text goes
//! where the stretch was.

use super::session::Heard;
use super::SAMPLES_PER_SECOND;
use crate::audio_toolkit::gain::FRAME;
use std::ops::Range;

/// A stretch with no word is a pause for up to this long.
const MIN_HOLE_SECONDS: f64 = 3.0;
/// Less speech than this in it is a breath or a click, which a second try
/// would only turn into an invented word.
const MIN_SPEECH_SECONDS: f64 = 1.0;

/// A stretch of a piece that holds speech and no word.
#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    /// Where it lies in the piece, in samples.
    pub audio: Range<usize>,
    /// How many words of the piece come before it.
    pub words_before: usize,
}

/// The holes of a piece of `piece_len` samples, whose frames are judged
/// speech or not by `speech`. Empty when the word times do not match
/// `heard.text`, or when there is no word at all: that piece is the same
/// audio again, and the model already runs a window that came back blank
/// once more.
pub fn find(heard: &Heard, piece_len: usize, speech: &[bool]) -> Vec<Hole> {
    if heard.words.is_empty() || heard.words.len() != heard.text.split_whitespace().count() {
        return Vec::new();
    }
    let duration = piece_len as f64 / SAMPLES_PER_SECOND as f64;
    let mut holes = Vec::new();
    // Where the last word ended: nothing was heard since.
    let mut quiet_since = 0.0_f64;
    for index in 0..=heard.words.len() {
        let next = heard.words.get(index);
        let until = next.map_or(duration, |word| word.start).min(duration);
        if until - quiet_since >= MIN_HOLE_SECONDS
            && speech_seconds(speech, quiet_since, until) >= MIN_SPEECH_SECONDS
        {
            holes.push(Hole {
                audio: sample_at(quiet_since, piece_len)..sample_at(until, piece_len),
                words_before: index,
            });
        }
        if let Some(word) = next {
            quiet_since = quiet_since.max(word.end).max(word.start);
        }
    }
    holes
}

fn sample_at(seconds: f64, piece_len: usize) -> usize {
    ((seconds * SAMPLES_PER_SECOND as f64).round() as usize).min(piece_len)
}

fn speech_seconds(speech: &[bool], from: f64, to: f64) -> f64 {
    let frames_per_second = SAMPLES_PER_SECOND as f64 / FRAME as f64;
    let first = ((from * frames_per_second) as usize).min(speech.len());
    let last = ((to * frames_per_second).ceil() as usize).min(speech.len());
    let speaking = speech[first..last.max(first)]
        .iter()
        .filter(|frame| **frame)
        .count();
    speaking as f64 / frames_per_second
}

/// `text` with each text of `fills` put before the word it names. The
/// words of `text` keep their order, separated by one space.
pub fn fill(text: &str, fills: &[(usize, String)]) -> String {
    if fills.is_empty() {
        return text.trim().to_string();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for index in 0..=words.len() {
        out.extend(
            fills
                .iter()
                .filter(|(before, _)| *before == index)
                .flat_map(|(_, filled)| filled.split_whitespace()),
        );
        out.extend(words.get(index));
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES_PER_SECOND: usize = SAMPLES_PER_SECOND / FRAME;

    /// What the model heard: words of `text` at the given times.
    fn heard(text: &str, times: &[(f64, f64)]) -> Heard {
        Heard {
            text: text.into(),
            words: times.iter().map(|(start, end)| *start..*end).collect(),
        }
    }

    fn seconds(count: f64) -> usize {
        (count * SAMPLES_PER_SECOND as f64).round() as usize
    }

    /// Frames of a piece `length` seconds long, speaking over `spans`.
    fn speaking(length: f64, spans: &[(f64, f64)]) -> Vec<bool> {
        (0..(length * FRAMES_PER_SECOND as f64).ceil() as usize)
            .map(|frame| {
                let at = frame as f64 / FRAMES_PER_SECOND as f64;
                spans.iter().any(|(from, to)| (*from..*to).contains(&at))
            })
            .collect()
    }

    #[test]
    fn a_first_sentence_the_model_skipped_is_a_hole_before_the_first_word() {
        let heard = heard("la fin", &[(8.3, 8.6), (8.6, 10.0)]);
        let speech = speaking(10.3, &[(0.0, 10.3)]);
        let holes = find(&heard, seconds(10.3), &speech);
        assert_eq!(holes.len(), 1, "{holes:?}");
        assert_eq!(holes[0].words_before, 0);
        assert_eq!(holes[0].audio.start, 0);
        assert!(
            (holes[0].audio.end as i64 - seconds(8.3) as i64).abs() < 2,
            "{holes:?}"
        );
    }

    #[test]
    fn a_last_sentence_the_model_skipped_is_a_hole_after_the_last_word() {
        let heard = heard("le début", &[(0.2, 0.9), (0.9, 6.7)]);
        let speech = speaking(10.3, &[(0.0, 10.3)]);
        let holes = find(&heard, seconds(10.3), &speech);
        assert_eq!(holes.len(), 1, "{holes:?}");
        assert_eq!(holes[0].words_before, 2);
        assert_eq!(holes[0].audio.start, seconds(6.7));
        assert_eq!(holes[0].audio.end, seconds(10.3));
    }

    #[test]
    fn a_stretch_in_the_middle_of_a_piece_is_a_hole_between_its_words() {
        let heard = heard("un deux trois", &[(0.0, 1.0), (1.0, 2.0), (7.0, 8.0)]);
        let speech = speaking(8.0, &[(0.0, 8.0)]);
        let holes = find(&heard, seconds(8.0), &speech);
        assert_eq!(
            holes,
            [Hole {
                audio: seconds(2.0)..seconds(7.0),
                words_before: 2
            }]
        );
    }

    #[test]
    fn a_pause_is_not_a_hole_and_neither_is_a_short_gap() {
        let words = heard("un deux", &[(0.0, 1.0), (7.0, 8.0)]);
        let pause = speaking(8.0, &[(0.0, 1.0), (7.0, 8.0)]);
        assert!(find(&words, seconds(8.0), &pause).is_empty());

        // Under 3 s, even when all of it is speech.
        let close = heard("un deux", &[(0.0, 1.0), (3.5, 4.5)]);
        let talking = speaking(4.5, &[(0.0, 4.5)]);
        assert!(find(&close, seconds(4.5), &talking).is_empty());
    }

    #[test]
    fn a_breath_in_a_long_gap_is_not_speech_enough_for_a_second_try() {
        let words = heard("un deux", &[(0.0, 1.0), (7.0, 8.0)]);
        let breath = speaking(8.0, &[(0.0, 1.0), (4.0, 4.6), (7.0, 8.0)]);
        assert!(find(&words, seconds(8.0), &breath).is_empty());
        let sentence = speaking(8.0, &[(0.0, 1.0), (4.0, 5.2), (7.0, 8.0)]);
        assert_eq!(find(&words, seconds(8.0), &sentence).len(), 1);
    }

    #[test]
    fn a_piece_without_a_word_or_without_times_is_never_a_hole() {
        let speech = speaking(10.0, &[(0.0, 10.0)]);
        let nothing = Heard::default();
        assert!(find(&nothing, seconds(10.0), &speech).is_empty());
        // A runtime that gives no times.
        let untimed = Heard {
            text: "bonjour à tous".into(),
            words: Vec::new(),
        };
        assert!(find(&untimed, seconds(10.0), &speech).is_empty());
        // A text rewritten after the times: they no longer name its words.
        let rewritten = heard("Kubernetes", &[(0.0, 1.0), (8.0, 9.0)]);
        assert!(find(&rewritten, seconds(10.0), &speech).is_empty());
    }

    #[test]
    fn a_hole_stays_inside_the_piece() {
        let heard = heard("un", &[(0.0, 1.0)]);
        let speech = speaking(5.0, &[(0.0, 5.0)]);
        let holes = find(&heard, seconds(5.0) - 7, &speech);
        assert_eq!(holes.len(), 1);
        assert_eq!(holes[0].audio.end, seconds(5.0) - 7);
    }

    #[test]
    fn what_was_heard_again_goes_where_the_hole_was() {
        assert_eq!(fill(" un deux ", &[]), "un deux");
        let fills = [(0, "zéro".to_string())];
        assert_eq!(fill("un deux", &fills), "zéro un deux");
        let fills = [(1, "entre les deux".to_string())];
        assert_eq!(fill("un deux", &fills), "un entre les deux deux");
        let fills = [(2, "trois".to_string())];
        assert_eq!(fill("un deux", &fills), "un deux trois");
        let fills = [(0, "a".to_string()), (2, "z".to_string())];
        assert_eq!(fill("un deux", &fills), "a un deux z");
        // A hole the model heard nothing in adds nothing.
        let fills = [(1, String::new())];
        assert_eq!(fill("un deux", &fills), "un deux");
    }
}
