//! Without headphones the microphone also hears the remote participants, so
//! their words would show twice: once under their name, once as "Me". A
//! sentence of the microphone track is such an echo when the system track
//! says the same words at the same moment, at least as loud. The user
//! talking over someone says different words, so it is kept.

use super::align::dp::{align, similarity, Step};
use super::align::TimedWord;
use super::chunking::FRAME_MS;
use super::merge::{Track, Unit};

/// System words this far around the sentence are compared with it.
const AROUND_MS: u64 = 1_500;
/// Share of the sentence's words found in the system track.
const MIN_OVERLAP: f32 = 0.6;
/// Shorter sentences ("Oui.") are too common to judge.
const MIN_WORDS: usize = 3;
/// The system track must be no quieter than the microphone by more than this.
const MAX_QUIETER_DB: f32 = 6.0;
/// Two words this alike count as the same word.
const SAME_WORD: f32 = 0.8;

/// The mean level of `[start_ms, end_ms)`, in dBFS, from 20 ms `levels`.
pub fn mean_level(levels: &[f32], start_ms: u64, end_ms: u64) -> Option<f32> {
    let first = (start_ms / FRAME_MS) as usize;
    let last = (end_ms.div_ceil(FRAME_MS) as usize).min(levels.len());
    let frames = levels
        .get(first..last)
        .filter(|frames| !frames.is_empty())?;
    // Averaged as power, so a loud word is not drowned by the pauses.
    let power: f32 =
        frames.iter().map(|db| 10f32.powf(db / 10.0)).sum::<f32>() / frames.len() as f32;
    Some(10.0 * power.max(1e-12).log10())
}

/// Whether the microphone sentence `unit` is an echo of the system track.
pub fn is_echo(
    unit: &Unit,
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> bool {
    let keys: Vec<&str> = unit
        .tokens
        .iter()
        .map(|token| token.key.as_str())
        .filter(|key| !key.is_empty())
        .collect();
    if keys.len() < MIN_WORDS {
        return false;
    }
    let from = unit.start_ms.saturating_sub(AROUND_MS);
    let to = unit.end_ms + AROUND_MS;
    let nearby: Vec<&str> = system_words
        .iter()
        .filter(|word| word.start_ms >= from && word.end_ms <= to)
        .map(|word| word.key.as_str())
        .collect();
    let Some(steps) = align(&keys, &nearby) else {
        return false;
    };
    let same = steps
        .iter()
        .filter(|step| match step {
            Step::Pair(m, s) => similarity(keys[*m], nearby[*s]) >= SAME_WORD,
            _ => false,
        })
        .count();
    if (same as f32) < MIN_OVERLAP * keys.len() as f32 {
        return false;
    }
    match (
        mean_level(mic_levels, unit.start_ms, unit.end_ms),
        mean_level(system_levels, unit.start_ms, unit.end_ms),
    ) {
        (Some(mic), Some(system)) => system >= mic - MAX_QUIETER_DB,
        _ => false,
    }
}

/// Removes the microphone sentences that echo the system track. Returns how
/// many were removed.
pub fn remove_echoes(
    units: &mut Vec<Unit>,
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> usize {
    let before = units.len();
    units.retain(|unit| {
        unit.track != Track::Mic || !is_echo(unit, system_words, mic_levels, system_levels)
    });
    before - units.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::tokens::{key, tokenize};

    fn mic(text: &str, start_ms: u64, end_ms: u64) -> Unit {
        Unit {
            track: Track::Mic,
            speaker: "me".to_string(),
            start_ms,
            end_ms,
            tokens: tokenize(text),
            low_confidence: false,
        }
    }

    fn heard(text: &str, start_ms: u64, step_ms: u64) -> Vec<TimedWord> {
        text.split(' ')
            .enumerate()
            .map(|(i, word)| TimedWord {
                key: key(word),
                start_ms: start_ms + i as u64 * step_ms,
                end_ms: start_ms + (i as u64 + 1) * step_ms,
            })
            .collect()
    }

    /// 20 s of levels: `db` everywhere.
    fn flat(db: f32) -> Vec<f32> {
        vec![db; 1_000]
    }

    #[test]
    fn a_sentence_the_system_track_said_at_the_same_time_is_an_echo() {
        let system = heard("on valide le budget jeudi prochain", 2_000, 400);
        let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
        // The microphone hears the speakers 10 dB below the system track.
        assert!(is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
        let mut units = vec![unit, mic("Moi je préfère lundi.", 6_000, 7_000)];
        assert_eq!(
            remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0)),
            1
        );
        assert_eq!(units.len(), 1);
    }

    #[test]
    fn talking_over_someone_is_kept() {
        let system = heard("on valide le budget jeudi prochain", 2_000, 400);
        let unit = mic("Attends je ne suis pas d'accord.", 2_100, 4_500);
        assert!(!is_echo(&unit, &system, &flat(-20.0), &flat(-30.0)));
    }

    #[test]
    fn short_sentences_are_kept() {
        let system = heard("oui merci", 2_000, 400);
        assert!(!is_echo(
            &mic("Oui merci.", 2_000, 2_800),
            &system,
            &flat(-40.0),
            &flat(-30.0)
        ));
    }

    #[test]
    fn the_same_words_much_quieter_on_the_system_track_are_kept() {
        // The user said it, and the remote side repeated it faintly.
        let system = heard("on valide le budget jeudi prochain", 2_000, 400);
        let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
        assert!(!is_echo(&unit, &system, &flat(-20.0), &flat(-35.0)));
    }

    #[test]
    fn the_same_words_at_another_moment_are_kept() {
        let system = heard("on valide le budget jeudi prochain", 30_000, 400);
        let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
        assert!(!is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
    }

    #[test]
    fn system_sentences_are_never_removed() {
        let system = heard("on valide le budget jeudi prochain", 2_000, 400);
        let mut unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
        unit.track = Track::System;
        let mut units = vec![unit];
        assert_eq!(
            remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0)),
            0
        );
    }

    #[test]
    fn the_mean_level_is_taken_as_power() {
        let mut levels = flat(-60.0);
        levels[50..100].iter_mut().for_each(|db| *db = -20.0);
        // Half the frames at -20 dB, half at -60: about -23 dB.
        let mean = mean_level(&levels, 1_000, 3_000).unwrap();
        assert!((mean - -23.0).abs() < 0.1, "{mean}");
        assert!(mean_level(&levels, 30_000, 31_000).is_none());
    }
}
