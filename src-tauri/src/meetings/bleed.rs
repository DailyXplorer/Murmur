//! Without headphones the microphone also hears the remote participants, so
//! their words would show twice: once under their name, once as "Me". A
//! sentence of the microphone track is such an echo when the system track
//! says the same words at the same moment, at least as loud. Only the
//! clauses that echo go, each judged on its own words: a reply Codex set off
//! with punctuation in the same sentence stays, whatever surrounds it. The
//! user talking over someone says different words, so it is kept.

use std::ops::Range;

use super::align::dp::{align, similarity, Step};
use super::align::TimedWord;
use super::chunking::FRAME_MS;
use super::merge::{Track, Unit};
use super::tokens::{Token, CLOSERS};

/// An echo is heard while the system track plays it, so a microphone
/// sentence is compared only with the words of a system sentence said at the
/// same time, give or take this much for the sentence and word times.
const SLACK_MS: u64 = 300;
/// Share of the sentence's words found in the system track.
const MIN_OVERLAP: f32 = 0.6;
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

/// The echoes of the system sentence said over `said_ms` in the microphone
/// sentence `unit`, in order; none when the sentence is not an echo. When in
/// doubt, the user's words stay: a clause goes only when it has at least
/// `MIN_CLAUSE_WORDS` words and the system track said every one of them
/// during that sentence. Every other clause stays, whatever its neighbours:
/// a reply ("d'accord, je regarde."), and a word the recognisers spell apart
/// ("à 15h."), which leaves some echo under "Me".
fn echoes(
    unit: &Unit,
    said_ms: (u64, u64),
    system_words: &[TimedWord],
    mic_levels: &[f32],
    system_levels: &[f32],
) -> Vec<Echo> {
    let Some(heard) = echoed_words(unit, said_ms, system_words, mic_levels, system_levels) else {
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

/// The tokens of the microphone sentence `unit` heard on the system track
/// while the user spoke and within the system sentence said over `said_ms`,
/// in order, if the sentence is an echo. What the remote side went on to say
/// after that sentence does not count, even while the user was still
/// speaking, nor what it said in it seconds before the user repeated it.
fn echoed_words(
    unit: &Unit,
    (said_start_ms, said_end_ms): (u64, u64),
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
                && word.start_ms < said_end_ms
                && word.end_ms > said_start_ms
        })
        .collect();
    let nearby_keys: Vec<&str> = nearby.iter().map(|word| word.key.as_str()).collect();
    let steps = align(&keys, &nearby_keys)?;
    let same: Vec<(usize, usize)> = steps
        .iter()
        .filter_map(|step| match *step {
            Step::Pair(m, s) if similarity(keys[m], nearby_keys[s]) >= SAME_WORD => Some((m, s)),
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

/// Of the system sentences timed `transcribed`, the one said over `unit`
/// that overlaps it most.
fn said_over(unit: &Unit, transcribed: &[(u64, u64)]) -> Option<(u64, u64)> {
    transcribed
        .iter()
        .copied()
        .filter(|&(start, end)| during(unit, start, end))
        .max_by_key(|&(start, end)| {
            end.min(unit.end_ms)
                .saturating_sub(start.max(unit.start_ms))
        })
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
        let found = match said_over(&unit, &transcribed) {
            Some(said_ms) if unit.track == Track::Mic => {
                echoes(&unit, said_ms, system_words, mic_levels, system_levels)
            }
            _ => Vec::new(),
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
        said_over(unit, &[sentence])
            .is_some_and(|said_ms| !echoes(unit, said_ms, system, mic_levels, levels).is_empty())
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

    /// What the system track said and when (after " + ", what it went on to
    /// say 200 ms later, during the microphone sentence; after " / ", what it
    /// said once that sentence was over), whether it has its own transcript
    /// there, what the microphone wrote from 2.1 s (or from the time after
    /// "@", in ms), the two levels, and the microphone sentences left. Every reported case stays.
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
        for &(said, at_ms, transcribed, written, mic_db, system_db, kept) in CORPUS {
            let end_ms = |start_ms: u64, text: &str| start_ms + tokenize(text).len() as u64 * 400;
            let (mic_ms, written) = written
                .strip_prefix('@')
                .and_then(|rest| rest.split_once(' '))
                .map_or((2_100, written), |(ms, rest)| (ms.parse().unwrap(), rest));
            let (said, then) = said.split_once(" / ").unwrap_or((said, ""));
            let (said, meanwhile) = said.split_once(" + ").unwrap_or((said, ""));
            let mut system = heard(said, at_ms, 400);
            if !meanwhile.is_empty() {
                system.extend(heard(meanwhile, end_ms(at_ms, said) + 200, 400));
            }
            if !then.is_empty() {
                system.extend(heard(then, end_ms(mic_ms, written) + 200, 400));
            }
            let mut units = vec![if transcribed {
                let mut unit = mic(said, at_ms, end_ms(at_ms, said));
                unit.track = Track::System;
                unit
            } else {
                Unit::placeholder(Track::System, 0, 600_000)
            }];
            units.push(mic(written, mic_ms, end_ms(mic_ms, written)));
            remove_echoes(&mut units, &system, &flat(mic_db), &flat(system_db));
            let left: Vec<String> = units
                .iter()
                .filter(|unit| unit.track == Track::Mic)
                .map(|unit| join(&unit.tokens))
                .collect();
            if left != kept {
                failures.push(format!("{written:?}: got {left:?}, want {kept:?}"));
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
