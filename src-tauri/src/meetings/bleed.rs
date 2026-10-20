//! Without headphones the microphone also hears the remote participants, so
//! their words would show twice: once under their name, once as "Me". A
//! sentence of the microphone track is such an echo when the system track
//! says the same words at the same moment, at least as loud. Only the
//! clauses that echo go, each judged on its own words: a reply Codex set off
//! with punctuation in the same sentence stays, whatever surrounds it. The
//! user talking over someone says different words, so it is kept.

use std::ops::Range;

use super::align::dp::{align_by, pair_cost, similarity, Step};
use super::align::TimedWord;
use super::chunking::FRAME_MS;
use super::merge::{Track, Unit};
use super::tokens::{Token, CLOSERS};

/// An echo is heard while the system track plays it, so a microphone
/// sentence is compared only with the system words said at the same time,
/// give or take this much for the word times.
const SLACK_MS: u64 = 300;
/// Share of the sentence's words found in the system track.
const MIN_OVERLAP: f32 = 0.6;
/// Of two copies of a word in a microphone sentence, the one said nearer the
/// system word is its echo: in "à dix heures, dix heures", the user repeats
/// the remote side's words. Small, so it only settles such ties.
const COST_PER_SECOND_APART: f32 = 0.01;
/// Of two copies equally near, as when the recogniser missed both, the echo
/// is the one that runs on with the other echoed words: the first in "à dix
/// heures, dix heures", said after "à", the second in "dix heures, dix
/// heures, c'est parfait", said before "c'est parfait". So each run of
/// echoed words costs what 10 ms apart does.
const COST_PER_RUN: f32 = 0.0001;
/// Shorter sentences ("Oui.") are too common to judge.
const MIN_WORDS: usize = 3;
/// Shorter clauses ("Oui,", "okay,") are too common to judge: they stay.
const MIN_CLAUSE_WORDS: usize = 2;
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

/// Whether `[start_ms, end_ms)` overlaps `unit`, within `SLACK_MS`.
fn during(unit: &Unit, start_ms: u64, end_ms: u64) -> bool {
    start_ms < unit.end_ms + SLACK_MS && end_ms + SLACK_MS > unit.start_ms
}

/// A run of clauses of a microphone sentence that echoes the system track.
struct Echo {
    tokens: Range<usize>,
    /// When the system track said them.
    start_ms: u64,
    end_ms: u64,
}

/// A token of a microphone sentence heard on the system track.
struct Heard {
    token: usize,
    start_ms: u64,
    end_ms: u64,
}

/// Whether Codex ended a clause after `token`: "vendredi," or "Oui.".
fn ends_clause(token: &Token) -> bool {
    token.sentence_end
        || token
            .text
            .trim_end_matches(CLOSERS)
            .ends_with([',', ';', ':', '，', '；', '：', '、'])
}

/// The token ranges of the clauses of `tokens`.
fn clauses(tokens: &[Token]) -> Vec<Range<usize>> {
    let mut clauses = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if ends_clause(token) || index + 1 == tokens.len() {
            clauses.push(start..index + 1);
            start = index + 1;
        }
    }
    clauses
}

/// The echoes of the system sentences timed `transcribed` in the microphone
/// sentence `unit`, in order; none when the sentence is not an echo. When in
/// doubt, the user's words stay: a clause goes only when it has at least
/// `MIN_CLAUSE_WORDS` words and the system track said every one of them
/// during the sentence. Every other clause stays, whatever its neighbours:
/// a reply ("d'accord, je regarde."), and a word the recognisers spell apart
/// ("à 15h."), which leaves some echo under "Me".
fn echoes(
    unit: &Unit,
    transcribed: &[(u64, u64)],
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> Vec<Echo> {
    let Some(heard) = echoed_words(unit, transcribed, system_words, mic_levels, system_levels)
    else {
        return Vec::new();
    };
    let echoed = |clause: &Range<usize>| {
        let keyed: Vec<usize> = clause
            .clone()
            .filter(|&index| !unit.tokens[index].key.is_empty())
            .collect();
        keyed.len() >= MIN_CLAUSE_WORDS
            && keyed
                .iter()
                .all(|&index| heard.iter().any(|heard| heard.token == index))
    };
    let mut runs: Vec<Range<usize>> = Vec::new();
    for clause in clauses(&unit.tokens).into_iter().filter(echoed) {
        match runs.last_mut() {
            Some(run) if run.end == clause.start => run.end = clause.end,
            _ => runs.push(clause),
        }
    }
    runs.into_iter()
        .filter_map(|tokens| {
            let mut within = heard.iter().filter(|heard| tokens.contains(&heard.token));
            let first = within.next()?;
            let end_ms = within.next_back().unwrap_or(first).end_ms;
            Some(Echo {
                start_ms: first.start_ms,
                end_ms,
                tokens,
            })
        })
        .collect()
}

/// When each token of `unit` may have been said, as a range. A word the
/// on-device recogniser heard was said at the middle of the time it heard
/// it; one it missed, anywhere between the words it heard around it, as a
/// time placed there would only be a guess. When it heard none of the
/// sentence, each word is at the middle of its share of the speaking time,
/// which misplaces words after a pause.
fn token_times(unit: &Unit) -> Vec<(u64, u64)> {
    if unit.tokens.iter().all(|token| token.heard_ms.is_none()) {
        let duration = unit.end_ms.saturating_sub(unit.start_ms);
        let total: usize = unit.tokens.iter().map(Token::weight).sum();
        let mut before = 0;
        return unit
            .tokens
            .iter()
            .map(|token| {
                // The token's middle, in half weights.
                let middle = 2 * before + token.weight();
                before += token.weight();
                let ms = unit.start_ms + duration * middle as u64 / (2 * total.max(1)) as u64;
                (ms, ms)
            })
            .collect();
    }
    let mut after = unit.start_ms;
    let mut times: Vec<(u64, u64)> = unit
        .tokens
        .iter()
        .map(|token| match token.heard_ms {
            Some((start_ms, end_ms)) => {
                after = end_ms;
                let middle = (start_ms + end_ms) / 2;
                (middle, middle)
            }
            None => (after, unit.end_ms),
        })
        .collect();
    let mut before = unit.end_ms;
    for (token, time) in unit.tokens.iter().zip(&mut times).rev() {
        match token.heard_ms {
            Some((start_ms, _)) => before = start_ms,
            None => time.1 = before,
        }
    }
    times
}

/// The tokens of the microphone sentence `unit` heard on the system track
/// while the user spoke, in order, if the sentence is an echo. Every system
/// sentence timed `transcribed` counts, as Codex may end one where the
/// microphone's copy has a comma; what the remote side said seconds before
/// the user repeated it does not.
fn echoed_words(
    unit: &Unit,
    transcribed: &[(u64, u64)],
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> Option<Vec<Heard>> {
    let (indices, keys): (Vec<usize>, Vec<&str>) = unit
        .tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| !token.key.is_empty())
        .map(|(index, token)| (index, token.key.as_str()))
        .unzip();
    if keys.len() < MIN_WORDS {
        return None;
    }
    let nearby: Vec<&TimedWord> = system_words
        .iter()
        .filter(|word| {
            during(unit, word.start_ms, word.end_ms)
                && transcribed
                    .iter()
                    .any(|&(start, end)| word.start_ms < end && word.end_ms > start)
        })
        .collect();
    let said_at = token_times(unit);
    let steps = align_by(keys.len(), nearby.len(), COST_PER_RUN, |m, s| {
        let word = nearby[s];
        let middle = (word.start_ms + word.end_ms) / 2;
        let (from, to) = said_at[indices[m]];
        let apart_ms = from.saturating_sub(middle) + middle.saturating_sub(to);
        pair_cost(keys[m], &word.key) + COST_PER_SECOND_APART * apart_ms as f32 / 1000.0
    })?;
    let same: Vec<(usize, usize)> = steps
        .iter()
        .filter_map(|step| match *step {
            Step::Pair(m, s) if similarity(keys[m], &nearby[s].key) >= SAME_WORD => Some((m, s)),
            _ => None,
        })
        .collect();
    if (same.len() as f32) < MIN_OVERLAP * keys.len() as f32 {
        return None;
    }
    let loud_enough = match (
        mean_level(mic_levels, unit.start_ms, unit.end_ms),
        mean_level(system_levels, unit.start_ms, unit.end_ms),
    ) {
        (Some(mic), Some(system)) => system >= mic - MAX_QUIETER_DB,
        _ => false,
    };
    loud_enough.then(|| {
        same.iter()
            .map(|&(m, s)| Heard {
                token: indices[m],
                start_ms: nearby[s].start_ms,
                end_ms: nearby[s].end_ms,
            })
            .collect()
    })
}

/// What is left of `unit` around its `echoes`: the user's own words, as when
/// Codex wrote "On t'envoie le rapport, d'accord." for the remote side's
/// sentence and the user's reply. Each piece is timed by where the system
/// track said the echoes on either side of it.
fn around(unit: Unit, echoes: &[Echo]) -> Vec<Unit> {
    let boundary = |ms: u64| ms.clamp(unit.start_ms, unit.end_ms);
    let mut pieces = Vec::with_capacity(echoes.len() + 1);
    let mut start = (0, unit.start_ms);
    for echo in echoes {
        pieces.push((start, (echo.tokens.start, boundary(echo.start_ms))));
        start = (echo.tokens.end, boundary(echo.end_ms));
    }
    pieces.push((start, (unit.tokens.len(), unit.end_ms)));
    pieces
        .into_iter()
        .map(|((from, start_ms), (to, end_ms))| (&unit.tokens[from..to], start_ms, end_ms))
        .filter(|(tokens, _, _)| tokens.iter().any(|token| !token.key.is_empty()))
        .map(|(tokens, start_ms, end_ms)| Unit {
            track: unit.track,
            speaker: unit.speaker.clone(),
            start_ms,
            end_ms,
            tokens: tokens.to_vec(),
            low_confidence: unit.low_confidence,
        })
        .collect()
}

/// Removes the parts of microphone sentences that echo the system track.
/// Returns how many echoes were removed. An echo is removed only where the
/// system track has its own transcript: a reply too short to send to Codex,
/// or a piece that failed, would otherwise be lost.
pub fn remove_echoes(
    units: &mut Vec<Unit>,
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> usize {
    let transcribed: Vec<(u64, u64)> = units
        .iter()
        .filter(|unit| unit.track == Track::System && !unit.is_placeholder())
        .map(|unit| (unit.start_ms, unit.end_ms))
        .collect();
    let mut removed = 0;
    let mut kept = Vec::with_capacity(units.len());
    for unit in units.drain(..) {
        let found = if unit.track == Track::Mic {
            echoes(&unit, &transcribed, system_words, mic_levels, system_levels)
        } else {
            Vec::new()
        };
        if found.is_empty() {
            kept.push(unit);
        } else {
            removed += 1;
            kept.extend(around(unit, &found));
        }
    }
    *units = kept;
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::align::timing::time_chunk;
    use crate::meetings::speakers;
    use crate::meetings::tokens::{join, key, tokenize};

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

    /// Whether `unit` echoes the system sentence of all the words in `system`.
    fn is_echo(unit: &Unit, system: &[TimedWord], mic_levels: &[f32], levels: &[f32]) -> bool {
        let sentence = (system[0].start_ms, system[system.len() - 1].end_ms);
        !echoes(unit, &[sentence], system, mic_levels, levels).is_empty()
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
        let mut said = mic("On valide le budget jeudi prochain.", 2_000, 4_400);
        said.track = Track::System;
        let mut units = vec![said, unit, mic("Moi je préfère lundi.", 6_000, 7_000)];
        assert_eq!(
            remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0)),
            1
        );
        assert_eq!(units.len(), 2);
    }

    #[test]
    fn an_echo_the_system_transcript_lacks_is_kept() {
        // A reply too short to send to Codex: its system chunk is silent,
        // so the microphone's copy is the only one in the transcript.
        let system = heard("oui ça me va", 2_000, 250);
        let unit = mic("Oui, ça me va.", 2_000, 3_000);
        assert!(is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
        let mut units = vec![unit, Unit::placeholder(Track::System, 0, 600_000)];
        assert_eq!(
            remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0)),
            0
        );
        assert_eq!(units.len(), 2);
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

    /// The text of `said` without its pauses, when each of its tokens was
    /// said from `start_ms` (400 ms a token, "~1500" a pause of 1.5 s), and
    /// whether the on-device recogniser heard it ("^" before a word it
    /// missed).
    fn schedule(said: &str, start_ms: u64) -> (String, Vec<(u64, u64)>, Vec<bool>) {
        let mut words = Vec::new();
        let mut times = Vec::new();
        let mut heard = Vec::new();
        let mut at_ms = start_ms;
        for word in said.split(' ') {
            if let Some(pause_ms) = word.strip_prefix('~') {
                at_ms += pause_ms.parse::<u64>().unwrap();
                continue;
            }
            let missed = word.strip_prefix('^');
            let word = missed.unwrap_or(word);
            for _ in tokenize(word) {
                times.push((at_ms, at_ms + 400));
                heard.push(missed.is_none());
                at_ms += 400;
            }
            words.push(word);
        }
        (words.join(" "), times, heard)
    }

    /// What the system track said and when (" | " between two sentences
    /// Codex transcribed apart, 200 ms apart; after " + ", what it went on to
    /// say 200 ms later, during the microphone sentence; after " / ", what it
    /// said once that sentence was over), whether it has its own transcript
    /// there, what the microphone wrote from 2.1 s (or from the time after
    /// "@", in ms), the two levels, and the microphone sentences left. Every
    /// reported case stays. Each row runs with the microphone's words timed
    /// from what the on-device recogniser heard of them, and again with only
    /// the sentence timed, unless the microphone pauses ("~"): an even spread
    /// cannot place it.
    #[allow(clippy::type_complexity)]
    const CORPUS: &[(&str, u64, bool, &str, f32, f32, &[&str])] = &[
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "On valide le budget jeudi prochain.",
            -40.0,
            -30.0,
            &[],
        ),
        (
            "we'll send you the report before friday",
            2_000,
            true,
            "We'll send you the report before Friday.",
            -40.0,
            -30.0,
            &[],
        ),
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "Attends je ne suis pas d'accord.",
            -20.0,
            -30.0,
            &["Attends je ne suis pas d'accord."],
        ),
        (
            "oui merci",
            2_000,
            true,
            "Oui merci.",
            -40.0,
            -30.0,
            &["Oui merci."],
        ),
        // The user said it, and the remote side repeated it faintly.
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "On valide le budget jeudi prochain.",
            -20.0,
            -35.0,
            &["On valide le budget jeudi prochain."],
        ),
        (
            "on valide le budget jeudi prochain",
            30_000,
            true,
            "On valide le budget jeudi prochain.",
            -40.0,
            -30.0,
            &["On valide le budget jeudi prochain."],
        ),
        // Round 27: the system piece was not transcribed, so the
        // microphone's copy is the only one.
        (
            "oui ça me va",
            2_000,
            false,
            "Oui, ça me va.",
            -40.0,
            -30.0,
            &["Oui, ça me va."],
        ),
        // Round 18: a CJK sentence is one token, too short to judge.
        (
            "我们周四确定预算",
            2_000,
            true,
            "我们周四确定预算。",
            -40.0,
            -30.0,
            &["我们周四确定预算。"],
        ),
        // Round 32: the user's reply in the same sentence as the echo.
        (
            "on t'envoie le rapport avant vendredi",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, parfait.",
            -40.0,
            -30.0,
            &["d'accord, parfait."],
        ),
        (
            "we'll send you the report before friday",
            2_000,
            true,
            "We'll send you the report before Friday, okay, sounds good.",
            -40.0,
            -30.0,
            &["okay, sounds good."],
        ),
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "Oui, on valide le budget jeudi prochain.",
            -40.0,
            -30.0,
            &["Oui,"],
        ),
        (
            "we'll send you the report before friday",
            2_000,
            true,
            "Sure, we'll send you the report before Friday.",
            -40.0,
            -30.0,
            &["Sure,"],
        ),
        // The remote side goes on right after the reply, with a word of the
        // reply ("le", "I'll"): that word is not part of the echo.
        (
            "on t'envoie le rapport avant vendredi / super et le budget",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
            -40.0,
            -30.0,
            &["d'accord, je le relis."],
        ),
        (
            "we'll send you the report before friday / great and i'll share",
            2_000,
            true,
            "We'll send you the report before Friday, okay, I'll read it.",
            -40.0,
            -30.0,
            &["okay, I'll read it."],
        ),
        // Round 33: the remote side goes on during the reply (after " + ").
        (
            "on t'envoie le rapport avant vendredi + et le budget aussi",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
            -40.0,
            -30.0,
            &["d'accord, je le relis."],
        ),
        (
            "we'll send you the report before friday + and i'll send the slides",
            2_000,
            true,
            "We'll send you the report before Friday, okay, I'll read it.",
            -40.0,
            -30.0,
            &["okay, I'll read it."],
        ),
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "On valide le budget, oui, jeudi prochain.",
            -40.0,
            -30.0,
            &["oui,"],
        ),
        (
            "we'll send you the report before friday",
            2_000,
            true,
            "We'll send you the report, right, before Friday.",
            -40.0,
            -30.0,
            &["right,"],
        ),
        // Review 3: the remote side goes on during the reply with a word of
        // it ("je", "I'll"); the reply stays.
        (
            "on t'envoie le rapport avant vendredi + et je te mets en copie",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je regarde.",
            -40.0,
            -30.0,
            &["d'accord, je regarde."],
        ),
        (
            "we'll send you the report before friday + and i'll copy you",
            2_000,
            true,
            "We'll send you the report before Friday, okay, I'll check.",
            -40.0,
            -30.0,
            &["okay, I'll check."],
        ),
        (
            "on t'envoie le rapport avant vendredi + et je te le renvoie",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
            -40.0,
            -30.0,
            &["d'accord, je le relis."],
        ),
        // Review 4: with headphones, the user repeats a time the remote side
        // said seconds earlier in the same long sentence; the repeat stays.
        (
            "on se voit jeudi à dix heures dans la grande salle avec toute l'équipe",
            2_000,
            true,
            "@6400 Jeudi, dix heures, ok.",
            -25.0,
            -20.0,
            &["Jeudi, dix heures, ok."],
        ),
        (
            "so we meet on thursday at ten in the big room with the whole team",
            2_000,
            true,
            "@6400 Thursday at ten, okay.",
            -25.0,
            -20.0,
            &["Thursday at ten, okay."],
        ),
        // Round 34: Codex ends a sentence of the system track where the
        // microphone's copy has a comma; the echo of both goes.
        (
            "on valide le budget | et le planning aussi",
            2_000,
            true,
            "On valide le budget, et le planning aussi.",
            -40.0,
            -30.0,
            &[],
        ),
        (
            "on valide le budget du projet | jeudi prochain",
            2_000,
            true,
            "On valide le budget du projet, jeudi prochain.",
            -40.0,
            -30.0,
            &[],
        ),
        (
            "we'll send the report | and the slides too",
            2_000,
            true,
            "We'll send the report, and the slides too.",
            -40.0,
            -30.0,
            &[],
        ),
        // The remote side's next sentence shares a word or two with the
        // reply; the reply stays.
        (
            "on t'envoie le rapport avant vendredi | et je te le renvoie",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
            -40.0,
            -30.0,
            &["d'accord, je le relis."],
        ),
        (
            "on t'envoie le rapport avant vendredi | et je te mets en copie",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je regarde.",
            -40.0,
            -30.0,
            &["d'accord, je regarde."],
        ),
        (
            "on t'envoie le rapport avant vendredi | et le budget aussi",
            2_000,
            true,
            "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
            -40.0,
            -30.0,
            &["d'accord, je le relis."],
        ),
        (
            "we'll send you the report before friday | and i'll copy you",
            2_000,
            true,
            "We'll send you the report before Friday, okay, I'll check.",
            -40.0,
            -30.0,
            &["okay, I'll check."],
        ),
        // Round 34: the user repeats the remote side's words in the same
        // sentence; the echo goes, the repeat stays.
        (
            "on se voit jeudi à dix heures",
            2_000,
            true,
            "On se voit jeudi à dix heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "we'll send you the report before friday",
            2_000,
            true,
            "We'll send you the report before Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        // Round 35: the remote side pauses before the words the user
        // repeats; the echo still goes, not the repeat.
        (
            "on se voit jeudi ~1500 à dix heures",
            2_000,
            true,
            "On se voit jeudi ~1500 à dix heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "on se voit jeudi ~300 à dix heures",
            2_000,
            true,
            "On se voit jeudi ~300 à dix heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "we'll send you the report ~1500 before friday",
            2_000,
            true,
            "We'll send you the report ~1500 before Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        (
            "we'll send you the report ~300 before friday",
            2_000,
            true,
            "We'll send you the report ~300 before Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        // Round 36: the recogniser missed the faint echo of the words the
        // user repeats ("^"), so their time is only known to lie between the
        // words it heard around them, whatever the user said before.
        (
            "on se voit jeudi ~300 à dix heures",
            2_000,
            true,
            "On se voit jeudi ~300 ^à ^dix ^heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "on se voit jeudi ~1500 à dix heures",
            2_000,
            true,
            "On se voit jeudi ~1500 ^à ^dix ^heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "on se voit jeudi ~3000 à dix heures",
            2_000,
            true,
            "On se voit jeudi ~3000 ^à ^dix ^heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "on se voit jeudi ~1500 à dix heures",
            2_000,
            true,
            "@1300 Bonjour. ~400 On se voit jeudi ~1500 ^à ^dix ^heures, dix heures, parfait.",
            -40.0,
            -30.0,
            &["Bonjour.", "dix heures, parfait."],
        ),
        (
            "we'll send you the report ~300 before friday",
            2_000,
            true,
            "We'll send you the report ~300 ^before ^Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        (
            "we'll send you the report ~1500 before friday",
            2_000,
            true,
            "We'll send you the report ~1500 ^before ^Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        (
            "we'll send you the report ~3000 before friday",
            2_000,
            true,
            "We'll send you the report ~3000 ^before ^Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        (
            "we'll send you the report ~1500 before friday",
            2_000,
            true,
            "@1300 Hello. ~400 We'll send you the report ~1500 ^before ^Friday, before Friday, great.",
            -40.0,
            -30.0,
            &["Hello.", "before Friday, great."],
        ),
        // Round 37: it missed the user's repeat too, so both copies may have
        // been said anywhere in the same stretch.
        (
            "on se voit jeudi ~1500 à dix heures",
            2_000,
            true,
            "@2000 On se voit jeudi ~1500 ^à ^dix ^heures, ^dix ^heures, parfait.",
            -40.0,
            -30.0,
            &["dix heures, parfait."],
        ),
        (
            "we'll send you the report ~1500 before friday",
            2_000,
            true,
            "@2000 We'll send you the report ~1500 ^before ^Friday, ^before ^Friday, great.",
            -40.0,
            -30.0,
            &["before Friday, great."],
        ),
        // Round 38: the other way round, the remote side repeats the user
        // before going on, and it missed both copies; the user's words stay.
        (
            "dix heures c'est parfait on valide le budget",
            3_800,
            true,
            "@2000 ^Dix ^heures, ~1000 ^dix ^heures, c'est parfait, on valide le budget.",
            -40.0,
            -30.0,
            &["Dix heures,"],
        ),
        (
            "dix heures c'est parfait on valide le budget",
            4_600,
            true,
            "@2000 Jeudi à ^dix ^heures, ~1000 ^dix ^heures, c'est parfait, on valide le budget.",
            -40.0,
            -30.0,
            &["Jeudi à dix heures,"],
        ),
        (
            "at ten that works we'll send the report",
            3_800,
            true,
            "@2000 ^At ^ten, ~1000 ^at ^ten, that works, we'll send the report.",
            -40.0,
            -30.0,
            &["At ten,"],
        ),
        (
            "at ten that works we'll send the report",
            4_600,
            true,
            "@2000 So Friday ^at ^ten, ~1000 ^at ^ten, that works, we'll send the report.",
            -40.0,
            -30.0,
            &["So Friday at ten,"],
        ),
        // Accepted echo left in the transcript. A clause goes only when the
        // system track said every one of its words, whatever its neighbours,
        // so these rows, which expected the echo gone, now keep part of it
        // under "Me". A time, an amount or a name set off between echoed
        // clauses no longer goes with them, nor a last clause the system
        // track ends after the microphone sentence ("devant le bureau."):
        (
            "on se retrouve jeudi quinze heures trente devant le bureau",
            2_000,
            true,
            "On se retrouve jeudi, 15h30, devant le bureau.",
            -40.0,
            -30.0,
            &["15h30, devant le bureau."],
        ),
        (
            "we meet thursday three thirty by the office",
            2_000,
            true,
            "We meet Thursday, 3:30, by the office.",
            -40.0,
            -30.0,
            &["3:30,"],
        ),
        (
            "le budget quinze mille euros est validé",
            2_000,
            true,
            "Le budget, 15 000 euros, est validé.",
            -40.0,
            -30.0,
            &["15 000 euros,"],
        ),
        (
            "the budget fifteen thousand dollars is approved",
            2_000,
            true,
            "The budget, $15,000, is approved.",
            -40.0,
            -30.0,
            &["$15,000, is approved."],
        ),
        (
            "écoute jean baptiste on valide le budget",
            2_000,
            true,
            "Écoute, Jean-Baptiste, on valide le budget.",
            -40.0,
            -30.0,
            &["Écoute, Jean-Baptiste,"],
        ),
        // A word the recognisers spell apart, or that Codex misheard, keeps
        // its whole clause:
        (
            "on valide le budget jeudi prochain",
            2_000,
            true,
            "On valide le bidule jeudi prochain.",
            -40.0,
            -30.0,
            &["On valide le bidule jeudi prochain."],
        ),
        (
            "on se voit à quinze heures",
            2_000,
            true,
            "On se voit à 15h.",
            -40.0,
            -30.0,
            &["On se voit à 15h."],
        ),
        (
            "on en parle avec jean baptiste",
            2_000,
            true,
            "On en parle avec Jean-Baptiste.",
            -40.0,
            -30.0,
            &["On en parle avec Jean-Baptiste."],
        ),
        (
            "we ship in q three",
            2_000,
            true,
            "We ship in Q3.",
            -40.0,
            -30.0,
            &["We ship in Q3."],
        ),
        (
            "let's ask mary jane",
            2_000,
            true,
            "Let's ask Mary-Jane.",
            -40.0,
            -30.0,
            &["Let's ask Mary-Jane."],
        ),
        (
            "on se voit à quinze heures devant le bureau",
            2_000,
            true,
            "On se voit à 15h, oui, devant le bureau.",
            -40.0,
            -30.0,
            &["On se voit à 15h, oui,"],
        ),
        (
            "we meet at three thirty by the office",
            2_000,
            true,
            "We meet at 3:30, right, by the office.",
            -40.0,
            -30.0,
            &["We meet at 3:30, right,"],
        ),
    ];

    #[test]
    fn echo_corpus() {
        let mut failures = Vec::new();
        let end_ms = |start_ms: u64, times: &[(u64, u64)]| times.last().map_or(start_ms, |t| t.1);
        let said_at = |said: &str, start_ms: u64| {
            let (text, times, _) = schedule(said, start_ms);
            let words: Vec<TimedWord> = tokenize(&text)
                .into_iter()
                .zip(&times)
                .map(|(token, &(start, end))| TimedWord {
                    key: token.key,
                    start_ms: start,
                    end_ms: end,
                })
                .collect();
            (text, words, end_ms(start_ms, &times))
        };
        for &(said, at_ms, transcribed, written, mic_db, system_db, kept) in CORPUS {
            let (mic_ms, written) = written
                .strip_prefix('@')
                .and_then(|rest| rest.split_once(' '))
                .map_or((2_100, written), |(ms, rest)| (ms.parse().unwrap(), rest));
            let (written, mic_times, mic_heard) = schedule(written, mic_ms);
            let mic_end_ms = end_ms(mic_ms, &mic_times);
            let (said, then) = said.split_once(" / ").unwrap_or((said, ""));
            let (said, meanwhile) = said.split_once(" + ").unwrap_or((said, ""));
            let mut system = Vec::new();
            let mut sentences = Vec::new();
            let mut next_ms = at_ms;
            for sentence in said.split(" | ") {
                let (text, words, sentence_end_ms) = said_at(sentence, next_ms);
                system.extend(words);
                let mut unit = mic(&text, next_ms, sentence_end_ms);
                unit.track = Track::System;
                sentences.push(unit);
                next_ms = sentence_end_ms + 200;
            }
            if !meanwhile.is_empty() {
                system.extend(said_at(meanwhile, next_ms).1);
            }
            if !then.is_empty() {
                system.extend(said_at(then, mic_end_ms + 200).1);
            }
            if !transcribed {
                sentences = vec![Unit::placeholder(Track::System, 0, 600_000)];
            }
            let paused = mic_times.windows(2).any(|pair| pair[0].1 < pair[1].0);
            let modes: &[bool] = if paused { &[true] } else { &[true, false] };
            for &measured in modes {
                let mut units = sentences.clone();
                if measured {
                    let words: Vec<TimedWord> = tokenize(&written)
                        .into_iter()
                        .zip(&mic_times)
                        .zip(&mic_heard)
                        .filter(|(_, &heard)| heard)
                        .map(|((token, &(start_ms, end_ms)), _)| TimedWord {
                            key: token.key,
                            start_ms,
                            end_ms,
                        })
                        .collect();
                    let timed = time_chunk(tokenize(&written), &words, mic_ms, mic_end_ms);
                    let me = vec!["me".to_string(); timed.tokens.len()];
                    units.extend(speakers::units(Track::Mic, timed.tokens, &me));
                } else {
                    units.push(mic(&written, mic_ms, mic_end_ms));
                }
                remove_echoes(&mut units, &system, &flat(mic_db), &flat(system_db));
                let left: Vec<String> = units
                    .iter()
                    .filter(|unit| unit.track == Track::Mic)
                    .map(|unit| join(&unit.tokens))
                    .collect();
                if left != kept {
                    let timed = if measured { "heard" } else { "spread" };
                    failures.push(format!(
                        "{written:?} ({timed}): got {left:?}, want {kept:?}"
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn a_reply_kept_from_an_echo_is_timed_from_the_echo() {
        let system = heard("on t'envoie le rapport avant vendredi", 2_000, 400);
        let mut said = mic("On t'envoie le rapport avant vendredi.", 2_000, 4_400);
        said.track = Track::System;
        let mut units = vec![
            said,
            mic("Oui, on t'envoie le rapport avant vendredi.", 1_500, 4_500),
            mic(
                "On t'envoie le rapport avant vendredi, d'accord, parfait.",
                2_000,
                5_400,
            ),
        ];
        remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0));
        let mic_times: Vec<(u64, u64)> = units[1..]
            .iter()
            .map(|unit| (unit.start_ms, unit.end_ms))
            .collect();
        assert_eq!(mic_times, [(1_500, 2_000), (4_400, 5_400)]);
    }

    #[test]
    fn a_reply_shares_no_word_with_what_the_remote_side_said_before_it() {
        // "then I'll call you" ends just before the user answers, and the
        // reply's "I'll" must not pull the echo back over it.
        let mut system = heard("then i'll call you", 200, 400);
        system.extend(heard("we'll send you the report before friday", 3_200, 400));
        let mut said = mic("We'll send you the report before Friday.", 3_200, 6_000);
        said.track = Track::System;
        let mut units = vec![
            said,
            mic(
                "Sure, I'll wait, we'll send you the report before Friday.",
                2_000,
                6_100,
            ),
        ];
        remove_echoes(&mut units, &system, &flat(-40.0), &flat(-30.0));
        let left: Vec<String> = units[1..].iter().map(|unit| join(&unit.tokens)).collect();
        assert_eq!(left, ["Sure, I'll wait,"]);
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
