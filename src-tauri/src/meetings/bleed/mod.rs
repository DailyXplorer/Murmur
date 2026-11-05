//! Without headphones the microphone also hears the remote participants, so
//! their words would show twice: once under their name, once as "Me". A
//! sentence of the microphone track is such an echo when the system track
//! says the same words at the same moment, at least as loud. Only the
//! clauses that echo go, each judged on its own words: a reply Codex set off
//! with punctuation in the same sentence stays, whatever surrounds it. The
//! user talking over someone says different words, so it is kept. Without
//! the on-device models (declined, or the analysis failed) the system
//! sentences are only placed to a few seconds, and the match is stricter.

use std::ops::Range;

use super::align::dp::{align_by, similarity, Step};
use super::align::TimedWord;
use super::chunking::FRAME_MS;
use super::merge::{Track, Unit};
use super::tokens::{Token, CLOSERS};

/// How the system words were timed, which sets how near and how alike a
/// microphone sentence must be to echo them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordTimes {
    /// Heard by the on-device recogniser, each to a fraction of a second.
    Heard,
    /// Without the on-device models: each system sentence spread over the
    /// track's speech (`spread`), as the microphone's are, which is good to
    /// a few seconds. A sentence then goes only when more of it, and a
    /// longer one, matches.
    Placed,
}

impl WordTimes {
    /// An echo is heard while the system track plays it, so a microphone
    /// sentence is compared only with the system words said at the same
    /// time, give or take this much for the times.
    fn slack_ms(self) -> u64 {
        match self {
            Self::Heard => 300,
            Self::Placed => 2_500,
        }
    }

    /// Share of the sentence's words found in the system track.
    fn min_overlap(self) -> f32 {
        match self {
            Self::Heard => 0.6,
            Self::Placed => 0.8,
        }
    }

    /// Shorter sentences ("Oui merci.") are too common to judge.
    fn min_words(self) -> usize {
        match self {
            Self::Heard => 3,
            Self::Placed => 4,
        }
    }
}

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
/// The pause the system track leaves before a word for the user to say
/// microphone words between it and the echoed words before. A run of echoed
/// words that starts after a shorter pause also costs what being that much
/// short of it apart does: pairing the second "dix" of "à dix heures, dix
/// heures" after "à" would have the user say the first one in no time.
const ROOM_MS: u64 = 300;
/// The fastest the user is taken to speak, per character of a word
/// (`Token::weight`): the words the recogniser missed take at least this.
const MIN_MS_PER_WEIGHT: u64 = 40;
/// The slowest the user is taken to speak, per character of a word.
const MAX_MS_PER_WEIGHT: u64 = 100;
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

/// Whether `[start_ms, end_ms)` overlaps `[from_ms, to_ms)` within
/// `slack_ms`.
fn during(from_ms: u64, to_ms: u64, start_ms: u64, end_ms: u64, slack_ms: u64) -> bool {
    start_ms < to_ms + slack_ms && end_ms + slack_ms > from_ms
}

/// When each token of `unit` before the first word the on-device
/// recogniser heard may have been said, in the microphone chunk starting
/// at `chunk_start_ms`; none when it heard none. The sentence's start was
/// placed back from that word at an average pace, so a token it missed
/// before it may have been said as early as it and the missed ones after it
/// take at `MAX_MS_PER_WEIGHT`, but not before the chunk, and as late as the
/// ones after it leave at `MIN_MS_PER_WEIGHT`. Both bounds take the user to
/// speak on to that word without a pause.
fn lead_times(unit: &Unit, chunk_start_ms: u64) -> Vec<(u64, u64)> {
    let Some((first, heard_ms)) = unit
        .tokens
        .iter()
        .enumerate()
        .find_map(|(index, token)| token.heard_ms.map(|(start_ms, _)| (index, start_ms)))
    else {
        return Vec::new();
    };
    let mut after = 0;
    let mut times: Vec<(u64, u64)> = unit.tokens[..first]
        .iter()
        .rev()
        .map(|token| {
            let to_ms = heard_ms.saturating_sub(after * MIN_MS_PER_WEIGHT);
            after += token.weight() as u64;
            let from_ms = heard_ms
                .saturating_sub(after * MAX_MS_PER_WEIGHT)
                .max(chunk_start_ms);
            (from_ms, to_ms)
        })
        .collect();
    times.reverse();
    times
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
/// sentence `unit`, cut from the chunk starting at `chunk_start_ms`, in
/// order; none when the sentence is not an echo. When in
/// doubt, the user's words stay: a clause goes only when it has at least
/// `MIN_CLAUSE_WORDS` words and the system track said every one of them
/// during the sentence. Every other clause stays, whatever its neighbours:
/// a reply ("d'accord, je regarde."), and a word the recognisers spell apart
/// ("à 15h."), which leaves some echo under "Me".
fn echoes(
    unit: &Unit,
    chunk_start_ms: u64,
    transcribed: &[(u64, u64)],
    system_words: &[TimedWord],
    times: WordTimes,
    mic_levels: &[f32],
    system_levels: &[f32],
) -> Vec<Echo> {
    let Some(heard) = echoed_words(
        unit,
        chunk_start_ms,
        transcribed,
        system_words,
        times,
        mic_levels,
        system_levels,
    ) else {
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
/// it; one it missed, between the words it heard around it, as a time
/// placed there would only be a guess, but leaving time for the missed
/// words on either side: said right after "à", the second "dix" of "à dix
/// heures, dix heures" would leave the first no time. When it heard none of
/// the sentence, each word is at the middle of its share of the speaking
/// time, which misplaces words after a pause.
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
    let mut times = Vec::with_capacity(unit.tokens.len());
    let mut after = unit.start_ms;
    let mut runs = unit
        .tokens
        .chunk_by(|a, b| a.heard_ms.is_none() && b.heard_ms.is_none())
        .peekable();
    while let Some(run) = runs.next() {
        if let Some((start_ms, end_ms)) = run[0].heard_ms {
            after = end_ms;
            let middle = (start_ms + end_ms) / 2;
            times.push((middle, middle));
            continue;
        }
        let before = runs
            .peek()
            .and_then(|next| next[0].heard_ms)
            .map_or(unit.end_ms, |(start_ms, _)| start_ms);
        times.extend(missed_times(run, after, before));
    }
    times
}

/// When each of the `missed` tokens, said between `from_ms` and `to_ms`, may
/// have been said: late enough for the ones before it, early enough for the
/// ones after it, at `MIN_MS_PER_WEIGHT` or as fast as the time between
/// needs.
fn missed_times(missed: &[Token], from_ms: u64, to_ms: u64) -> Vec<(u64, u64)> {
    let total: u64 = missed.iter().map(|token| token.weight() as u64).sum();
    let span_ms = to_ms.saturating_sub(from_ms);
    // How long `half` half weights take.
    let lasts = |half: u64| (half * MIN_MS_PER_WEIGHT).min(span_ms * half / total.max(1)) / 2;
    let mut before = 0;
    missed
        .iter()
        .map(|token| {
            let weight = token.weight() as u64;
            let after = total - before - weight;
            let time = (
                from_ms + lasts(2 * before + weight),
                to_ms.saturating_sub(lasts(2 * after + weight)),
            );
            before += weight;
            time
        })
        .collect()
}

/// The tokens of the microphone sentence `unit` heard on the system track
/// while the user spoke, in order, if the sentence is an echo. Every system
/// sentence timed `transcribed` counts, as Codex may end one where the
/// microphone's copy has a comma; what the remote side said seconds before
/// the user repeated it does not. An echo is heard as the system track says
/// it, so a word the recogniser missed before the first one it heard is
/// echoed only by a system word said when the user may have said it: the
/// user confirming a date said seconds before, as in "Mercredi prochain
/// quatorze heures, c'est noté.", is not echoing it, even while the remote
/// side goes on talking.
fn echoed_words(
    unit: &Unit,
    chunk_start_ms: u64,
    transcribed: &[(u64, u64)],
    system_words: &[TimedWord],
    times: WordTimes,
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
    if keys.len() < times.min_words() {
        return None;
    }
    let lead = lead_times(unit, chunk_start_ms);
    // Whether the system track said `word` when the user may have said the
    // token at `index`.
    let within = |index: usize, word: &TimedWord| {
        let (from_ms, to_ms) = lead
            .get(index)
            .copied()
            .unwrap_or((unit.start_ms, unit.end_ms));
        during(from_ms, to_ms, word.start_ms, word.end_ms, times.slack_ms())
    };
    let nearby: Vec<&TimedWord> = system_words
        .iter()
        .filter(|word| {
            indices.iter().any(|&index| within(index, word))
                && transcribed
                    .iter()
                    .any(|&(start, end)| word.start_ms < end && word.end_ms > start)
        })
        .collect();
    // A system word said when the user could not have said the microphone
    // word is no echo of it, however alike they are.
    let alike = |m: usize, s: usize| {
        if within(indices[m], nearby[s]) {
            similarity(keys[m], &nearby[s].key)
        } else {
            0.0
        }
    };
    let said_at = token_times(unit);
    let run_cost = |m: usize, s: usize| {
        // A run from the sentence's first word skips no microphone words.
        let pause_ms = match s.checked_sub(1) {
            Some(previous) if m > 0 => nearby[s].start_ms.saturating_sub(nearby[previous].end_ms),
            _ => ROOM_MS,
        };
        COST_PER_RUN + COST_PER_SECOND_APART * ROOM_MS.saturating_sub(pause_ms) as f32 / 1000.0
    };
    let steps = align_by(keys.len(), nearby.len(), run_cost, |m, s| {
        let word = nearby[s];
        let middle = (word.start_ms + word.end_ms) / 2;
        let (from, to) = said_at[indices[m]];
        let apart_ms = from.saturating_sub(middle) + middle.saturating_sub(to);
        1.0 - 2.0 * alike(m, s) + COST_PER_SECOND_APART * apart_ms as f32 / 1000.0
    })?;
    // A missed token's earliest time is at the slowest pace already, so a
    // system word that ended before it, paired only within the slack, is
    // not its echo: "Next Tuesday, perfect." said 200 ms after the remote
    // side's "next Tuesday" is the user's.
    let echoed = |m: usize, s: usize| {
        alike(m, s) >= SAME_WORD
            && lead
                .get(indices[m])
                .is_none_or(|&(from_ms, _)| nearby[s].end_ms > from_ms)
    };
    let same: Vec<(usize, usize)> = steps
        .iter()
        .filter_map(|step| match *step {
            Step::Pair(m, s) if echoed(m, s) => Some((m, s)),
            _ => None,
        })
        .collect();
    if (same.len() as f32) < times.min_overlap() * keys.len() as f32 {
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
/// or a piece that failed, would otherwise be lost. `mic_chunk_starts` are
/// where the microphone track was cut into the pieces sent to Codex, and
/// `times` how `system_words` were timed.
pub fn remove_echoes(
    units: &mut Vec<Unit>,
    mic_chunk_starts: &[u64],
    system_words: &[TimedWord],
    times: WordTimes,
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
            // A sentence never runs across chunks, so it starts in the last
            // one cut before it.
            let chunk_start_ms = mic_chunk_starts
                .iter()
                .copied()
                .filter(|&start_ms| start_ms <= unit.start_ms)
                .max()
                .unwrap_or(0);
            echoes(
                &unit,
                chunk_start_ms,
                &transcribed,
                system_words,
                times,
                mic_levels,
                system_levels,
            )
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
mod tests;
